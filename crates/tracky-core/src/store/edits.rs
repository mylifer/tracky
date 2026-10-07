//! Düzenlemeleri geri alma ve atanmamış süreyi gözden geçirme.
//!
//! Geri alma: bir düzenlemeden (atama, silme, elle kayıt) önce etkilenecek oturumların
//! durumu saklanır ([`EditSnapshot`]); geri alınınca o duruma dönülür ve düzenlemenin bölerek
//! ya da ekleyerek oluşturduğu satırlar silinir. Her değişiklik yeni bir `updated_at` alır;
//! eşitleme geri almayı da diğer bilgisayarlara taşır.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{FOREIGN_LIVE_WINDOW_MS, OVERLAPS, Result, Store, StoreError, from_ms, ms};
use crate::classify::{Classifier, NO_PROJECT, Rule, TagKind};
use crate::inbox::{self, RulePreview, Unassigned};
use crate::model::{IDLE_APP_ID, MANUAL_APP_ID};

/// Yoksayılan atanmamış süre grupları (`site:…`, `app:…`).
const IGNORED_UNASSIGNED_KEY: &str = "ignored_unassigned";

/// Bir oturum satırının düzenlemeyle değişebilecek alanları.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowState {
    id: String,
    device_id: String,
    started_at: i64,
    ended_at: i64,
    category_id: Option<String>,
    project_id: Option<String>,
}

/// Bir düzenlemeden önceki durum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSnapshot {
    /// Görüntünün alındığı an (ms).
    at: i64,
    rows: Vec<RowState>,
    /// Düzenleme satır bölebilir ya da ekleyebilirse bu aralıkta yeni satırlar aranır.
    extent: Option<(i64, i64)>,
}

impl EditSnapshot {
    /// Görüntünün kapsadığı zaman aralığı (ms); satırsız ve aralıksızsa `None`.
    fn span(&self) -> Option<(i64, i64)> {
        self.extent.or_else(|| {
            let a = self.rows.iter().map(|r| r.started_at).min()?;
            let b = self.rows.iter().map(|r| r.ended_at).max()?;
            Some((a, b))
        })
    }

    /// İki düzenleme aynı süreye dokunuyor mu. Geri alma sondan başa olmalı: eski düzenleme
    /// önce geri alınırsa yenisinin geri alınması, eskinin geri getirdiği satırları kendi
    /// eklediği satır sanıp silerdi.
    pub fn touches(&self, other: &EditSnapshot) -> bool {
        match (self.span(), other.span()) {
            (Some((a, b)), Some((c, d))) => a < d && c < b,
            _ => false,
        }
    }
}

impl Store {
    /// `[from, to)` ile kesişen oturumların durumu (aralığa atama, silme, elle kayıt öncesi).
    pub fn snapshot_range(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<EditSnapshot> {
        let rows = self.row_states(
            &format!("deleted_at IS NULL AND {OVERLAPS}"),
            params![ms(from), ms(to)],
        )?;
        let start = rows.iter().map(|r| r.started_at).fold(ms(from), i64::min);
        let end = rows.iter().map(|r| r.ended_at).fold(ms(to), i64::max);
        Ok(EditSnapshot {
            at: ms(Utc::now()),
            rows,
            extent: Some((start, end)),
        })
    }

    /// Belirli oturumların durumu (satır bölmeyen düzenlemeler öncesi).
    pub fn snapshot_sessions(&self, ids: &[Uuid]) -> Result<EditSnapshot> {
        let mut rows = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let list = chunk
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(",");
            rows.extend(self.row_states(&format!("id IN ({list})"), params![])?);
        }
        Ok(EditSnapshot {
            at: ms(Utc::now()),
            rows,
            extent: None,
        })
    }

    fn row_states(&self, condition: &str, args: impl rusqlite::Params) -> Result<Vec<RowState>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, device_id, started_at, ended_at, category_id, project_id
             FROM sessions WHERE {condition}"
        ))?;
        let rows = stmt.query_map(args, |r| {
            Ok(RowState {
                id: r.get(0)?,
                device_id: r.get(1)?,
                started_at: r.get(2)?,
                ended_at: r.get(3)?,
                category_id: r.get(4)?,
                project_id: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Görüntüdeki duruma döner. Süren oturum geri alınırken uzamış olabilir; bitişi kısalmaz.
    pub fn restore_snapshot(&self, snap: &EditSnapshot) -> Result<()> {
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        if let Some((start, end)) = snap.extent {
            let known: HashSet<&str> = snap.rows.iter().map(|r| r.id.as_str()).collect();
            let own = self.device_id.to_string();
            // Düzenleme yalnızca bu cihazın elle kaydını ya da görüntüdeki bir satırın
            // parçasını (aynı cihaz, satırın eski aralığı içinde) ekler. Bu arada başka
            // cihazdan gelen kayıt da aynı aralıkta yeni görünür; o silinmemeli.
            let created: Vec<String> = self
                .conn
                .prepare(
                    "SELECT id, device_id, started_at, ended_at FROM sessions
                     WHERE deleted_at IS NULL AND started_at < ?2 AND ended_at > ?1
                       AND started_at < ?3 AND updated_at >= ?3",
                )?
                .query_map(params![start, end, snap.at], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|(id, device, a, b)| {
                    !known.contains(id.as_str())
                        && (*device == own
                            || snap.rows.iter().any(|r| {
                                r.device_id == *device && r.started_at <= *a && *b <= r.ended_at
                            }))
                })
                .map(|(id, ..)| id)
                .collect();
            for id in created {
                self.conn.execute(
                    "UPDATE sessions SET deleted_at = ?2, state_at = ?2,
                         updated_at = MAX(?2, updated_at + 1)
                     WHERE id = ?1",
                    params![id, now],
                )?;
            }
        }
        for r in &snap.rows {
            self.conn.execute(
                "UPDATE sessions SET started_at = ?2, ended_at = MAX(?3, ended_at),
                     category_id = ?4, project_id = ?5, deleted_at = NULL, state_at = ?6,
                     updated_at = MAX(?6, updated_at + 1)
                 WHERE id = ?1",
                params![
                    r.id,
                    r.started_at,
                    r.ended_at,
                    r.category_id,
                    r.project_id,
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Oturumlara elle proje verir (`None`: kurallara döner). Başka bilgisayarda süren oturuma
    /// dokunulmaz (o bilgisayar satırı yeniden yazıp değişikliği ezerdi). Değişen satır sayısı.
    pub fn set_project_for(&self, ids: &[Uuid], project_id: Option<&str>) -> Result<usize> {
        if let Some(id) = project_id.filter(|id| *id != NO_PROJECT) {
            self.require_tag(id, TagKind::Project)?;
        }
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        let mut n = 0;
        for id in ids {
            n += self.conn.execute(
                "UPDATE sessions SET project_id = ?2, state_at = ?3, updated_at = MAX(?3, updated_at + 1)
                 WHERE id = ?1 AND deleted_at IS NULL AND project_id IS NOT ?2
                   AND NOT (device_id != ?4 AND ended_at > ?5
                            AND substr(app_id, 1, length(?6)) != ?6 AND app_id != ?7)",
                params![
                    id.to_string(),
                    project_id,
                    now,
                    self.device_id.to_string(),
                    now - FOREIGN_LIVE_WINDOW_MS,
                    format!("{MANUAL_APP_ID}/"),
                    IDLE_APP_ID,
                ],
            )?;
        }
        // Raporda bilerek atanan süre, projenin silinmiş satırında kalsa da yeniden önerilir.
        if let Some(project) = project_id.filter(|id| *id != NO_PROJECT) {
            for id in ids {
                if let Some((from, to)) = self
                    .conn
                    .query_row(
                        "SELECT started_at, ended_at FROM sessions WHERE id = ?1 AND project_id = ?2",
                        params![id.to_string(), project],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .optional()?
                {
                    self.forget_dismissed(project, from_ms(from), from_ms(to))?;
                }
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` aralığının atanmamış süresi, gruplanmış.
    pub fn unassigned(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Unassigned> {
        let sessions = self.merged_sessions_with_idle_between(from, to)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let ignored: HashSet<String> = self.ignored_unassigned()?.into_iter().collect();
        Ok(inbox::unassigned(
            &sessions,
            &classifier,
            &ignored,
            from,
            to,
        ))
    }

    /// Bir gruptaki (ve verilirse yalnızca o başlıktaki) atanmamış oturumlar.
    pub fn unassigned_sessions(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        key: &str,
        title: Option<&str>,
    ) -> Result<Vec<Uuid>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut ids: Vec<Uuid> = self
            .merged_sessions_between(from, to)?
            .into_iter()
            .filter(|s| {
                !s.is_idle()
                    && s.ended_at > from
                    && s.started_at < to
                    && inbox::is_unassigned(s, &classifier)
                    && inbox::group_of(s).0 == key
                    && title.is_none_or(|t| inbox::item_title(s) == t)
            })
            .map(|s| s.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// `[from, to)` aralığında aynı uygulama ve pencere başlığındaki çalışma oturumları
    /// (takvim bloğundaki bir pencereyi projeye atamak için; projesi olsa da olmasa da).
    pub fn window_sessions(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        app_id: &str,
        title: &str,
    ) -> Result<Vec<Uuid>> {
        let mut ids: Vec<Uuid> = self
            .merged_sessions_between(from, to)?
            .into_iter()
            .filter(|s| {
                !s.is_idle()
                    && s.ended_at > from
                    && s.started_at < to
                    && s.app_id == app_id
                    && s.title == title
            })
            .map(|s| s.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    pub fn ignored_unassigned(&self) -> Result<Vec<String>> {
        Ok(self
            .setting::<Vec<String>>(IGNORED_UNASSIGNED_KEY)?
            .unwrap_or_default())
    }

    /// Grubu atanmamış süre listesinde gösterme (`false`: yeniden göster).
    pub fn set_unassigned_ignored(&self, key: &str, ignored: bool) -> Result<()> {
        let mut keys = self.ignored_unassigned()?;
        keys.retain(|k| k != key);
        if ignored {
            keys.push(key.to_string());
        }
        self.save_setting(IGNORED_UNASSIGNED_KEY, &keys)
    }

    /// Kural eklenseydi son `days` günde ne değişirdi?
    pub fn preview_rule(&self, rule: &Rule, now: DateTime<Utc>, days: i64) -> Result<RulePreview> {
        if rule.pattern.trim().is_empty() {
            return Ok(RulePreview::default());
        }
        let from = now - Duration::days(days);
        let sessions = self.merged_sessions_between(from, now)?;
        let mut rule = rule.clone();
        if rule.field == crate::classify::RuleField::Domain {
            match crate::url_util::normalize_pattern(&rule.pattern) {
                Some(p) => rule.pattern = p,
                None => return Ok(RulePreview::default()),
            }
        }
        rule.pattern = rule.pattern.trim().to_string();
        Ok(inbox::preview_rule(
            &sessions,
            &self.tags()?,
            &self.rules()?,
            &rule,
            from,
            now,
        ))
    }

    /// Silinmiş kuralı (aynı kimlikle) geri getirir.
    pub fn restore_rule(&self, rule: &Rule) -> Result<()> {
        self.upsert_rule(rule)?;
        self.conn.execute(
            "UPDATE rules SET deleted_at = NULL, updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NOT NULL",
            params![rule.id, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Silinmiş etiketi ve onunla birlikte silinen kuralları geri getirir.
    pub fn restore_tag(&self, id: &str, deleted_at: DateTime<Utc>) -> Result<()> {
        let now = ms(Utc::now());
        let n = self.conn.execute(
            "UPDATE tags SET deleted_at = NULL, updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NOT NULL",
            params![id, now],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid("etiket geri getirilemedi".into()));
        }
        self.conn.execute(
            "UPDATE rules SET deleted_at = NULL, updated_at = MAX(?3, updated_at + 1)
             WHERE tag_id = ?1 AND deleted_at = ?2",
            params![id, ms(deleted_at), now],
        )?;
        Ok(())
    }

    /// Kuralı kimliğiyle bulur (silinmiş olsa da).
    pub fn rule_by_id(&self, id: &str) -> Result<Option<Rule>> {
        use rusqlite::OptionalExtension;
        let row = self
            .conn
            .query_row(
                "SELECT id, tag_id, field, pattern FROM rules WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.and_then(|(id, tag_id, field, pattern)| {
            crate::classify::RuleField::parse(&field).map(|field| Rule {
                id,
                tag_id,
                field,
                pattern,
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{RuleField, Tag};
    use crate::model::Session;

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + min * 60, 0).unwrap()
    }

    fn session(title: &str, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: "com.apple.Safari".into(),
            app_name: "Safari".into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
        }
    }

    fn project(store: &Store, name: &str) -> String {
        let tag = Tag {
            id: Uuid::new_v4().to_string(),
            kind: TagKind::Project,
            name: name.into(),
            color: 1,
        };
        store.upsert_tag(&tag, 0).unwrap();
        tag.id
    }

    fn state(store: &Store) -> Vec<(i64, i64, Option<String>)> {
        let mut v: Vec<_> = store
            .sessions_between(t(-1000), t(1000))
            .unwrap()
            .into_iter()
            .map(|s| {
                (
                    (s.started_at - t(0)).num_minutes(),
                    (s.ended_at - t(0)).num_minutes(),
                    s.project_id,
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn range_edits_can_be_undone() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        store.upsert_session(&session("a", 0, 60)).unwrap();
        store.upsert_session(&session("b", 60, 90)).unwrap();
        let before = state(&store);

        // Projeye atama (oturum ortadan bölünür).
        let snap = store.snapshot_range(t(30), t(70)).unwrap();
        store.set_project_between(t(30), t(70), Some(&p)).unwrap();
        assert_ne!(state(&store), before);
        store.restore_snapshot(&snap).unwrap();
        assert_eq!(state(&store), before);

        // Silme.
        let snap = store.snapshot_range(t(10), t(20)).unwrap();
        store.delete_between(t(10), t(20)).unwrap();
        assert_ne!(state(&store), before);
        store.restore_snapshot(&snap).unwrap();
        assert_eq!(state(&store), before);

        // Elle kayıt (boşta süreyi değiştirir).
        store
            .upsert_session(&Session::idle(t(100), t(160)))
            .unwrap();
        let before = state(&store);
        let snap = store.snapshot_range(t(110), t(130)).unwrap();
        store
            .add_manual_session("Toplantı", t(110), t(130), None, Some(&p))
            .unwrap();
        assert_ne!(state(&store), before);
        store.restore_snapshot(&snap).unwrap();
        assert_eq!(state(&store), before);
    }

    #[test]
    fn undo_keeps_rows_pulled_from_another_device() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        let set_device = |id: &Uuid| {
            store
                .conn()
                .execute(
                    "UPDATE sessions SET device_id = ?1 WHERE id = ?2",
                    params![Uuid::new_v4().to_string(), id.to_string()],
                )
                .unwrap();
        };
        // Başka cihazın bitmiş oturumu: bölünen parçaları geri almada silinmeli.
        let foreign = session("a", 0, 60);
        store.upsert_session(&foreign).unwrap();
        set_device(&foreign.id);
        let before = state(&store);
        let snap = store.snapshot_range(t(30), t(40)).unwrap();
        store.set_project_between(t(30), t(40), Some(&p)).unwrap();
        // Düzenlemeden sonra üçüncü bir cihazdan aralığa düşen kayıt gelir: kalmalı.
        let pulled = session("b", 20, 50);
        store.upsert_session(&pulled).unwrap();
        set_device(&pulled.id);
        store.restore_snapshot(&snap).unwrap();
        let mut expected = before;
        expected.push((20, 50, None));
        expected.sort();
        assert_eq!(state(&store), expected);
    }

    #[test]
    fn edits_nest_in_one_transaction_and_roll_back_together() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        let s = session("a", 0, 60);
        store.upsert_session(&s).unwrap();
        let before = state(&store);

        // Kendi işlemini açan düzenlemeler dış işlemin içinde de çalışır; sonradan gelen hata
        // hepsini geri alır.
        let out: Result<()> = store.atomic(|store| {
            store.set_project_for(&[s.id], Some(&p))?;
            store.set_project_between(t(10), t(20), None)?;
            assert_ne!(state(store), before);
            Err(StoreError::Invalid("dur".into()))
        });
        assert!(out.is_err());
        assert_eq!(state(&store), before);

        // İçteki başarısız düzenleme yalnızca kendini geri alır; dış işlem devam edebilir.
        let snap = store.snapshot_sessions(&[s.id]).unwrap();
        store
            .atomic(|store| {
                store.set_project_for(&[s.id], Some(&p))?;
                assert!(store.set_project_for(&[s.id], Some("yok")).is_err());
                Ok(())
            })
            .unwrap();
        assert_eq!(state(&store), vec![(0, 60, Some(p.clone()))]);
        store.atomic(|store| store.restore_snapshot(&snap)).unwrap();
        assert_eq!(state(&store), before);
    }

    #[test]
    fn unassigned_groups_can_be_assigned_ignored_and_undone() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        store.upsert_session(&session("Plan", 0, 30)).unwrap();
        store.upsert_session(&session("Plan", 40, 50)).unwrap();
        store.upsert_session(&session("Haberler", 50, 70)).unwrap();
        let out = store.unassigned(t(0), t(100)).unwrap();
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].key, "app:com.apple.Safari");
        assert_eq!(out.groups[0].seconds, 60 * 60);

        let ids = store
            .unassigned_sessions(t(0), t(100), "app:com.apple.Safari", Some("Plan"))
            .unwrap();
        assert_eq!(ids.len(), 2);
        let snap = store.snapshot_sessions(&ids).unwrap();
        assert_eq!(store.set_project_for(&ids, Some(&p)).unwrap(), 2);
        assert_eq!(
            store.unassigned(t(0), t(100)).unwrap().total_seconds,
            20 * 60
        );
        store.restore_snapshot(&snap).unwrap();
        assert_eq!(
            store.unassigned(t(0), t(100)).unwrap().total_seconds,
            60 * 60
        );

        store
            .set_unassigned_ignored("app:com.apple.Safari", true)
            .unwrap();
        assert!(store.unassigned(t(0), t(100)).unwrap().groups.is_empty());
        store
            .set_unassigned_ignored("app:com.apple.Safari", false)
            .unwrap();
        assert_eq!(store.unassigned(t(0), t(100)).unwrap().groups.len(), 1);
    }

    #[test]
    fn deleted_tags_and_rules_can_be_restored() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        let rule = Rule {
            id: Uuid::new_v4().to_string(),
            tag_id: p.clone(),
            field: RuleField::Title,
            pattern: "kum".into(),
        };
        store.upsert_rule(&rule).unwrap();
        store.delete_rule(&rule.id).unwrap();
        assert!(store.rules().unwrap().iter().all(|r| r.id != rule.id));
        let found = store.rule_by_id(&rule.id).unwrap().unwrap();
        store.restore_rule(&found).unwrap();
        assert!(store.rules().unwrap().iter().any(|r| r.id == rule.id));

        let at = store.delete_tag(&p).unwrap();
        assert!(store.tags().unwrap().iter().all(|t| t.id != p));
        store.restore_tag(&p, at).unwrap();
        assert!(store.tags().unwrap().iter().any(|t| t.id == p));
        assert!(store.rules().unwrap().iter().any(|r| r.id == rule.id));
    }

    #[test]
    fn window_sessions_find_one_window_whatever_its_project() {
        let store = Store::open_in_memory().unwrap();
        let p = project(&store, "kum");
        let mut assigned = session("LOY-214", 0, 10);
        assigned.project_id = Some(p.clone());
        let other = session("Gelen kutusu", 10, 20);
        let later = session("LOY-214", 20, 30);
        for s in [&assigned, &other, &later] {
            store.upsert_session(s).unwrap();
        }
        let mut found = store
            .window_sessions(t(0), t(25), "com.apple.Safari", "LOY-214")
            .unwrap();
        found.sort();
        let mut expected = vec![assigned.id, later.id];
        expected.sort();
        assert_eq!(found, expected);
        store.set_project_for(&found, None).unwrap();
        assert_eq!(
            state(&store),
            [(0, 10, None), (10, 20, None), (20, 30, None)]
        );
    }
}
