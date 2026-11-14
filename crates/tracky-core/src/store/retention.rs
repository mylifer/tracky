//! Eski kayıtların sadeleştirilmesi: belli bir tarihten eski oturumların pencere başlığı ve
//! adres yolu silinir; süre, uygulama, alan adı, kategori ve proje kalır.
//!
//! Sınıflandırma sorgu anında yapıldığından ([`crate::classify`]) başlığı silmek, başlık ya da
//! adres kuralıyla projeye düşen eski süreyi projesiz bırakırdı. Proje arşivindeki gibi önce
//! oturumun o anki kategori ve projesi oturuma yazılır ("dondurulur"), sonra başlık silinir:
//! rapor toplamları değişmez. Bunun bedeli, dondurulan oturumlara sonradan eklenen ya da
//! değiştirilen kuralların artık uygulanmaması; aramada da eski başlıklar bulunmaz.

use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{Result, Store, ms};
use crate::classify::{Classifier, NO_PROJECT};

impl Store {
    /// `before`'dan önce biten (silinmemiş, boşta olmayan) oturumların başlık ve adresini siler,
    /// kategori ve projesini dondurur. Değişen oturum sayısını döndürür; değişenler eşitlenir.
    /// Tek işlemde: yarıda kalırsa hiçbir şey değişmez.
    pub fn prune_details(&self, before: DateTime<Utc>) -> Result<usize> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let sessions: Vec<_> = self
            .sessions_between(DateTime::<Utc>::UNIX_EPOCH, before)?
            .into_iter()
            .filter(|s| {
                s.ended_at <= before && !s.is_idle() && (!s.title.is_empty() || s.url.is_some())
            })
            .collect();
        if sessions.is_empty() {
            return Ok(0);
        }
        // Kimlikler `sessions_between`'de boş gelir; satırlar aynı sıra ve anahtarla bulunur.
        let mut ids = self.conn.prepare(
            "SELECT id FROM sessions
             WHERE deleted_at IS NULL AND started_at = ?1 AND ended_at = ?2 AND app_id = ?3
               AND title = ?4 AND url IS ?5",
        )?;
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        let mut changed = 0;
        for s in &sessions {
            let class = classifier.classify(s);
            let category = class.category.or_else(|| s.category_id.clone());
            let project = match s.project_id.as_deref() {
                Some(NO_PROJECT) => s.project_id.clone(),
                _ => class.project.or_else(|| s.project_id.clone()),
            };
            let assigned = category != s.category_id || project != s.project_id;
            let rows: Vec<String> = ids
                .query_map(
                    params![ms(s.started_at), ms(s.ended_at), s.app_id, s.title, s.url],
                    |r| r.get(0),
                )?
                .collect::<rusqlite::Result<_>>()?;
            for id in rows {
                changed += self.conn.execute(
                    "UPDATE sessions SET title = '', url = NULL, category_id = ?2, project_id = ?3,
                            state_at = CASE WHEN ?4 THEN ?5 ELSE state_at END,
                            updated_at = MAX(?5, updated_at + 1)
                     WHERE id = ?1",
                    params![id, category, project, assigned, now],
                )?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Silinen verinin yerini diskte boşaltır (sadeleştirmeden sonra). İşlem içinde çağrılamaz.
    pub fn compact(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    use super::*;
    use crate::classify::{Rule, RuleField, Tag, TagKind};
    use crate::model::Session;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, hour, 0, 0).unwrap()
    }

    fn session(app: &str, title: &str, url: Option<&str>, start: DateTime<Utc>) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.into(),
            title: title.into(),
            url: url.map(str::to_string),
            domain: url.and_then(crate::url_util::domain_of),
            started_at: start,
            ended_at: start + Duration::hours(1),
            category_id: None,
            project_id: None,
            block_from: None,
            passive: false,
        }
    }

    fn project(store: &Store, id: &str) {
        let tag = Tag {
            id: id.into(),
            kind: TagKind::Project,
            name: id.into(),
            color: 1,
        };
        store.upsert_tag(&tag, 0).unwrap();
    }

    fn rule(store: &Store, id: &str, field: RuleField, pattern: &str, tag: &str) {
        let rule = Rule {
            id: id.into(),
            tag_id: tag.into(),
            field,
            pattern: pattern.into(),
        };
        store.upsert_rule(&rule).unwrap();
    }

    fn project_seconds(store: &Store, project: &str) -> i64 {
        let classifier = Classifier::new(&store.tags().unwrap(), &store.rules().unwrap());
        store
            .sessions_between(at(1, 0), at(20, 0))
            .unwrap()
            .iter()
            .filter(|s| classifier.classify(s).project.as_deref() == Some(project))
            .map(|s| (s.ended_at - s.started_at).num_seconds())
            .sum()
    }

    #[test]
    fn old_titles_go_but_totals_stay() {
        let store = Store::open_in_memory().unwrap();
        project(&store, "portal");
        rule(&store, "r1", RuleField::Title, "LOY-", "portal");
        rule(
            &store,
            "r2",
            RuleField::Domain,
            "github.com/firma/portal",
            "portal",
        );
        let old = [
            session("code", "LOY-214 ödeme", None, at(2, 9)),
            session(
                "safari",
                "PR #12",
                Some("https://github.com/firma/portal/pull/12"),
                at(2, 11),
            ),
            session("mail", "Gelen kutusu", None, at(2, 13)),
        ];
        let recent = session("code", "LOY-300 sepet", None, at(15, 9));
        for s in old.iter().chain([&recent]) {
            store.upsert_session(s).unwrap();
        }
        let before = project_seconds(&store, "portal");
        assert_eq!(before, 3 * 3600);

        assert_eq!(store.prune_details(at(10, 0)).unwrap(), 3);
        store.compact().unwrap();

        // Toplam değişmez; eski başlık ve adres yolu gider, alan adı ve uygulama kalır.
        assert_eq!(project_seconds(&store, "portal"), before);
        let all = store.sessions_between(at(1, 0), at(20, 0)).unwrap();
        let olds: Vec<_> = all.iter().filter(|s| s.started_at < at(10, 0)).collect();
        assert!(olds.iter().all(|s| s.title.is_empty() && s.url.is_none()));
        assert_eq!(olds[1].domain.as_deref(), Some("github.com"));
        assert_eq!(olds[2].app_id, "mail");
        // Projeye düşmeyen eski oturum atanmamış kalır.
        assert_eq!(olds[2].project_id, None);
        // Yeni oturuma dokunulmaz.
        let new = all.iter().find(|s| s.started_at == at(15, 9)).unwrap();
        assert_eq!(new.title, "LOY-300 sepet");
        assert_eq!(new.project_id, None);

        // Tekrar çalıştırmak bir şey değiştirmez.
        assert_eq!(store.prune_details(at(10, 0)).unwrap(), 0);
    }

    #[test]
    fn explicit_no_project_and_manual_choices_are_kept() {
        let store = Store::open_in_memory().unwrap();
        project(&store, "portal");
        project(&store, "diger");
        rule(&store, "r1", RuleField::Title, "LOY-", "portal");
        // Elle "projesiz" denen ve elle başka projeye verilen süre, kurala uysa da öyle kalır.
        let a = session("code", "LOY-1", None, at(2, 9));
        let b = session("code", "LOY-2", None, at(2, 11));
        store.upsert_session(&a).unwrap();
        store.upsert_session(&b).unwrap();
        store.set_project_for(&[a.id], Some(NO_PROJECT)).unwrap();
        store.set_project_for(&[b.id], Some("diger")).unwrap();
        assert_eq!(store.prune_details(at(10, 0)).unwrap(), 2);
        let all = store.sessions_between(at(1, 0), at(20, 0)).unwrap();
        assert_eq!(all[0].project_id.as_deref(), Some(NO_PROJECT));
        assert_eq!(all[1].project_id.as_deref(), Some("diger"));
    }
}
