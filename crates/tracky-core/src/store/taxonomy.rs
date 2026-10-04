//! Sınıflandırma: kategoriler, projeler, müşteriler, kurallar ve öneriler.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{Result, Store, StoreError, ms};
use crate::classify::{Client, DEFAULT_CATEGORIES, Rule, RuleField, Tag, TagKind, default_id};
use crate::suggest::{self, Suggestions};

const DEFAULTS_SEEDED_KEY: &str = "default_tags_seeded";

/// Yoksayılan öneri anahtarları.
const DISMISSED_SUGGESTIONS_KEY: &str = "dismissed_suggestions";
/// Öneriler bu kadar günlük geçmişe bakar.
const SUGGEST_DAYS: i64 = 14;

impl Store {
    /// İlk açılışta varsayılan kategorileri ekler (kullanıcı silerse geri gelmez).
    pub(super) fn seed_default_tags(&self) -> Result<()> {
        if self.setting::<bool>(DEFAULTS_SEEDED_KEY)?.is_some() {
            return Ok(());
        }
        for (position, (name, color, apps, titles)) in DEFAULT_CATEGORIES.iter().enumerate() {
            // Kimlikler adlardan türetilir: her cihaz aynı varsayılanları aynı
            // kimlikle üretir, senkronizasyonda kopya oluşmaz.
            let tag = Tag {
                id: default_id(&format!("category:{name}")),
                kind: TagKind::Category,
                name: name.to_string(),
                color: *color,
            };
            self.upsert_tag(&tag, position as i64)?;
            let patterns = apps
                .iter()
                .map(|p| (RuleField::App, p))
                .chain(titles.iter().map(|p| (RuleField::Title, p)));
            for (field, pattern) in patterns {
                self.upsert_rule(&Rule {
                    id: default_id(&format!("rule:{name}:{}:{pattern}", field.as_str())),
                    tag_id: tag.id.clone(),
                    field,
                    pattern: pattern.to_string(),
                })?;
            }
        }
        // Varsayılanlar "en eski sürüm" sayılır: başka bir cihazdaki gerçek bir
        // düzenleme, sonradan kurulan cihazın tohumuna her zaman üstün gelir.
        self.conn
            .execute_batch("UPDATE tags SET updated_at = 0; UPDATE rules SET updated_at = 0;")?;
        self.save_setting(DEFAULTS_SEEDED_KEY, &true)
    }

    pub(super) fn require_tag(&self, id: &str, kind: TagKind) -> Result<()> {
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT kind FROM tags WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        match found {
            Some(k) if k == kind.as_str() => Ok(()),
            _ => Err(StoreError::Invalid(format!("etiket bulunamadı: {id}"))),
        }
    }

    pub fn tags(&self) -> Result<Vec<Tag>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, color FROM tags
             WHERE deleted_at IS NULL ORDER BY kind, position, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u8>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, kind, name, color) = row?;
            let kind = TagKind::parse(&kind)
                .ok_or_else(|| StoreError::Invalid(format!("bilinmeyen etiket türü: {kind}")))?;
            Ok(Tag {
                id,
                kind,
                name,
                color,
            })
        })
        .collect()
    }

    /// Ekler ya da günceller; `position` yalnızca eklemede kullanılır.
    pub fn upsert_tag(&self, tag: &Tag, position: i64) -> Result<()> {
        if !(1..=8).contains(&tag.color) {
            return Err(StoreError::Invalid(format!(
                "renk 1-8 olmalı: {}",
                tag.color
            )));
        }
        self.conn.execute(
            "INSERT INTO tags (id, kind, name, color, position, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                name = excluded.name, color = excluded.color,
                updated_at = MAX(excluded.updated_at, tags.updated_at + 1)",
            params![
                tag.id,
                tag.kind.as_str(),
                tag.name.trim(),
                tag.color,
                position,
                ms(Utc::now())
            ],
        )?;
        Ok(())
    }

    /// Müşteriler, sıralı.
    pub fn clients(&self) -> Result<Vec<Client>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name FROM clients WHERE deleted_at IS NULL ORDER BY position, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Client {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn upsert_client(&self, client: &Client, position: i64) -> Result<()> {
        if client.name.trim().is_empty() {
            return Err(StoreError::Invalid("müşteri adı boş olamaz".into()));
        }
        self.conn.execute(
            "INSERT INTO clients (id, name, position, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET
                name = excluded.name, deleted_at = NULL,
                updated_at = MAX(excluded.updated_at, clients.updated_at + 1)",
            params![client.id, client.name.trim(), position, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Müşteriyi yumuşak siler; projeleri silinmez, müşterisiz kalır.
    pub fn delete_client(&self, id: &str) -> Result<()> {
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        self.conn.execute(
            "UPDATE clients SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1) WHERE id = ?1",
            params![id, now],
        )?;
        self.conn.execute(
            "UPDATE tags SET client_id = NULL, updated_at = MAX(?2, updated_at + 1)
             WHERE client_id = ?1",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Proje → müşteri (yalnızca müşterisi olan ve müşterisi silinmemiş projeler).
    pub fn project_clients(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.client_id FROM tags t JOIN clients c ON c.id = t.client_id
             WHERE t.kind = 'project' AND t.deleted_at IS NULL AND c.deleted_at IS NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Projeyi müşteriye bağlar (`None`: müşterisiz).
    pub fn set_project_client(&self, project_id: &str, client_id: Option<&str>) -> Result<()> {
        self.require_tag(project_id, TagKind::Project)?;
        if let Some(c) = client_id {
            let found: Option<i64> = self
                .conn
                .query_row(
                    "SELECT 1 FROM clients WHERE id = ?1 AND deleted_at IS NULL",
                    [c],
                    |r| r.get(0),
                )
                .optional()?;
            if found.is_none() {
                return Err(StoreError::Invalid(format!("müşteri bulunamadı: {c}")));
            }
        }
        self.conn.execute(
            "UPDATE tags SET client_id = ?2, updated_at = MAX(?3, updated_at + 1)
             WHERE id = ?1 AND client_id IS NOT ?2",
            params![project_id, client_id, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Yumuşak siler; kuralları da birlikte silinir. Silinme anı ([`Store::restore_tag`]).
    pub fn delete_tag(&self, id: &str) -> Result<DateTime<Utc>> {
        let at = Utc::now();
        let now = ms(at);
        self.conn.execute(
            "UPDATE tags SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1) WHERE id = ?1",
            params![id, now],
        )?;
        self.conn.execute(
            "UPDATE rules SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
             WHERE tag_id = ?1 AND deleted_at IS NULL",
            params![id, now],
        )?;
        Ok(super::from_ms(now))
    }

    /// Etkin kurallar: arşivdeki projelerin kuralları sınıflandırmaya girmez
    /// ([`Store::archive_project`]).
    pub fn rules(&self) -> Result<Vec<Rule>> {
        self.rules_where(false)
    }

    /// Arşivdekiler dahil tüm kurallar (örn. öneriler arşivdeki projenin sözcüklerini
    /// yeni proje diye önermesin).
    fn rules_where(&self, with_archived: bool) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tag_id, field, pattern FROM rules
             WHERE deleted_at IS NULL
               AND (?1 OR NOT EXISTS (SELECT 1 FROM tags t
                                      WHERE t.id = rules.tag_id AND t.archived_at IS NOT NULL))
             ORDER BY position, rowid",
        )?;
        let rows = stmt.query_map([with_archived], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        // Daha yeni bir sürümün eşitlediği, bu sürümün tanımadığı kural türü atlanır: tek bir
        // kural yüzünden raporların hepsi açılmaz olmasın.
        rows.filter_map(|row| match row {
            Ok((id, tag_id, field, pattern)) => RuleField::parse(&field).map(|field| {
                Ok(Rule {
                    id,
                    tag_id,
                    field,
                    pattern,
                })
            }),
            Err(e) => Some(Err(e.into())),
        })
        .collect()
    }

    pub fn upsert_rule(&self, rule: &Rule) -> Result<()> {
        let normalized;
        let pattern = match rule.field {
            RuleField::Domain => {
                normalized =
                    crate::url_util::normalize_pattern(&rule.pattern).ok_or_else(|| {
                        StoreError::Invalid(format!(
                            "geçerli bir web adresi değil: {}",
                            rule.pattern
                        ))
                    })?;
                normalized.as_str()
            }
            _ => rule.pattern.trim(),
        };
        if pattern.is_empty() {
            return Err(StoreError::Invalid("kural deseni boş olamaz".into()));
        }
        self.conn.execute(
            "INSERT INTO rules (id, tag_id, field, pattern, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                tag_id = excluded.tag_id, field = excluded.field,
                pattern = excluded.pattern,
                updated_at = MAX(excluded.updated_at, rules.updated_at + 1)",
            params![
                rule.id,
                rule.tag_id,
                rule.field.as_str(),
                pattern,
                ms(Utc::now())
            ],
        )?;
        Ok(())
    }

    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE rules SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1) WHERE id = ?1",
            params![id, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Bir uygulamayı bir kategoriye atar: mevcut uygulama kurallarını kaldırıp yenisini ekler.
    /// `tag_id: None` uygulamayı kategorisiz bırakır.
    pub fn assign_app_category(&self, app_id: &str, tag_id: Option<&str>) -> Result<()> {
        let tags = self.tags()?;
        let is_category = |id: &str| {
            tags.iter()
                .any(|t| t.id == id && t.kind == TagKind::Category)
        };
        // Kategori verilirken yalnızca bu uygulamanın kendi (tam) kuralları kalkar: önek
        // kuralları (`com.jetbrains.*`) başka uygulamaları da kapsar, onlara dokunulmaz;
        // tam kural sınıflandırmada önek kuralından önce gelir. Kategorisiz bırakmak için
        // uygulamayı kapsayan önek kuralları da kaldırılmalıdır.
        for rule in self.rules()? {
            if rule.field == RuleField::App
                && is_category(&rule.tag_id)
                && (tag_id.is_none() || !rule.pattern.ends_with('*'))
                && rule.matches(app_id, "")
            {
                self.delete_rule(&rule.id)?;
            }
        }
        if let Some(tag_id) = tag_id {
            self.upsert_rule(&Rule {
                id: Uuid::new_v4().to_string(),
                tag_id: tag_id.to_string(),
                field: RuleField::App,
                pattern: app_id.to_string(),
            })?;
        }
        Ok(())
    }

    /// Son iki haftanın oturumlarından proje ve kategori önerileri.
    pub fn suggestions(&self, now: DateTime<Utc>) -> Result<Suggestions> {
        let sessions =
            self.merged_sessions_between(now - chrono::Duration::days(SUGGEST_DAYS), now)?;
        let dismissed: HashSet<String> = self
            .setting::<Vec<String>>(DISMISSED_SUGGESTIONS_KEY)?
            .unwrap_or_default()
            .into_iter()
            .collect();
        // Arşivdeki projelerin kuralları da verilir: onlara uyan süre "zaten bir projede"
        // sayılır, arşivlenen iş yeni proje diye yeniden önerilmez.
        Ok(suggest::suggest(
            &sessions,
            &self.tags()?,
            &self.rules_where(true)?,
            &dismissed,
        ))
    }

    /// Öneriyi bir daha gösterme.
    pub fn dismiss_suggestion(&self, key: &str) -> Result<()> {
        let mut keys = self
            .setting::<Vec<String>>(DISMISSED_SUGGESTIONS_KEY)?
            .unwrap_or_default();
        if !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
            self.save_setting(DISMISSED_SUGGESTIONS_KEY, &keys)?;
        }
        Ok(())
    }

    /// Önerilen projeyi ekler: proje etiketi ve adıyla bir başlık kuralı.
    pub fn accept_project_suggestion(&self, name: &str) -> Result<Tag> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("proje adı boş olamaz".into()));
        }
        let tags = self.tags()?;
        // Arayüzdeki gibi: önce hiç kullanılmamış, yoksa en az kullanılan renk.
        let color = (1..=8u8)
            .min_by_key(|c| tags.iter().filter(|t| t.color == *c).count())
            .unwrap_or(1);
        let tag = Tag {
            id: Uuid::new_v4().to_string(),
            kind: TagKind::Project,
            name: name.to_string(),
            color,
        };
        self.upsert_tag(&tag, tags.len() as i64)?;
        self.upsert_rule(&Rule {
            id: Uuid::new_v4().to_string(),
            tag_id: tag.id.clone(),
            field: RuleField::Title,
            pattern: name.to_string(),
        })?;
        Ok(tag)
    }

    /// Önerilen kategori kuralını ekler (uygulama ya da başlık).
    pub fn accept_category_suggestion(
        &self,
        field: RuleField,
        pattern: &str,
        category_id: &str,
    ) -> Result<()> {
        self.require_tag(category_id, TagKind::Category)?;
        match field {
            RuleField::App => self.assign_app_category(pattern, Some(category_id)),
            RuleField::Title | RuleField::Domain => self.upsert_rule(&Rule {
                id: Uuid::new_v4().to_string(),
                tag_id: category_id.to_string(),
                field,
                pattern: pattern.to_string(),
            }),
        }
    }
}

/// Bir etiketin arşiv ve bütçe bilgisi.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagExtras {
    pub archived_at: Option<DateTime<Utc>>,
    /// Sözleşme bütçesi (adam-gün).
    pub budget_days: Option<f64>,
}

/// Bütçe 0'dan büyük, makul bir adam-gün olmalı (`None`: bütçe yok).
fn check_budget(days: Option<f64>) -> Result<Option<f64>> {
    match days {
        Some(d) if !(d.is_finite() && d > 0.0 && d <= 100_000.0) => Err(StoreError::Invalid(
            format!("bütçe 0'dan büyük bir adam-gün olmalı: {d}"),
        )),
        _ => Ok(days),
    }
}

impl Store {
    /// Etiket başına arşiv ve bütçe (yalnızca silinmemiş etiketler).
    pub fn tag_extras(&self) -> Result<HashMap<String, TagExtras>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, archived_at, budget_days FROM tags
             WHERE deleted_at IS NULL AND (archived_at IS NOT NULL OR budget_days IS NOT NULL)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                TagExtras {
                    archived_at: r.get::<_, Option<i64>>(1)?.map(super::from_ms),
                    budget_days: r.get(2)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Arşivdeki projeler.
    pub fn archived_projects(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tags
             WHERE kind = 'project' AND deleted_at IS NULL AND archived_at IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Projeyi arşivler: seçicilerde görünmez, kuralları yeni oturumları sınıflandırmaz,
    /// önerilere ve bildirimlere girmez; geçmişi ve rapor toplamları korunur.
    ///
    /// Sınıflandırma kayıt anında değil sorgu anında yapılır ([`crate::classify`]). Bu yüzden
    /// kuralları yalnızca devre dışı bırakmak geçmişi de değiştirirdi: kurala uyduğu için
    /// projeye düşen eski oturumlar projesiz kalırdı. Kuralların oturumun zamanına göre
    /// (arşivden önce/sonra) uygulanması da her sınıflandırma yerine arşiv bilgisini taşımayı
    /// gerektirirdi. Onun yerine arşivlerken geçmiş "dondurulur": kurala uyarak bu projeye
    /// düşen oturumlara proje elle verilmiş gibi yazılır (`project_id`), sonra kuralları
    /// [`Store::rules`]'tan çıkar. Böylece bütün raporlar (eşitlenen diğer cihazlar dahil)
    /// aynı toplamı gösterir, arşivden sonraki oturumlar ise projeye düşmez. Arşivden
    /// çıkarınca kurallar yeniden işler; dondurulan oturumlar zaten aynı projededir.
    ///
    /// Arşivden önce başka cihazda kaydedilip henüz eşitlenmemiş oturumlar dondurulamaz;
    /// eşitlenince projesiz görünürler (projeyi arşivden çıkarıp yeniden arşivlemek düzeltir).
    /// Dönen görüntü geri almak içindir ([`Store::restore_snapshot`]).
    pub fn archive_project(&self, id: &str) -> Result<super::EditSnapshot> {
        self.require_tag(id, TagKind::Project)?;
        let classifier = crate::classify::Classifier::new(&self.tags()?, &self.rules()?);
        let ids: Vec<Uuid> = self
            .sessions_between(DateTime::<Utc>::UNIX_EPOCH, DateTime::<Utc>::MAX_UTC)?
            .into_iter()
            .filter(|s| {
                !s.is_idle()
                    && s.project_id.as_deref() != Some(id)
                    && s.project_id.as_deref() != Some(crate::classify::NO_PROJECT)
                    && classifier.classify(s).project.as_deref() == Some(id)
            })
            .map(|s| s.id)
            .collect();
        let snap = self.snapshot_sessions(&ids)?;
        // `set_project_for` kendi işlemini açtığı için tek işleme alınamaz; arşiv işareti
        // konamasa da dondurulan oturumlar zaten kurala göre aynı projededir.
        self.set_project_for(&ids, Some(id))?;
        self.set_tag_archived_at(id, Some(Utc::now()))?;
        Ok(snap)
    }

    /// Projeyi arşivden çıkarır; önceki arşiv anını döndürür (geri almak için).
    pub fn unarchive_project(&self, id: &str) -> Result<Option<DateTime<Utc>>> {
        self.require_tag(id, TagKind::Project)?;
        let prev = self.tag_extras()?.remove(id).and_then(|e| e.archived_at);
        self.set_tag_archived_at(id, None)?;
        Ok(prev)
    }

    /// Arşiv anını doğrudan yazar (`None`: arşivde değil); geri alma için.
    pub fn set_tag_archived_at(&self, id: &str, at: Option<DateTime<Utc>>) -> Result<()> {
        self.conn.execute(
            "UPDATE tags SET archived_at = ?2, updated_at = MAX(?3, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NULL AND archived_at IS NOT ?2",
            params![id, at.map(ms), ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Projenin sözleşme bütçesi (adam-gün; `None` kaldırır).
    pub fn set_project_budget(&self, id: &str, days: Option<f64>) -> Result<()> {
        self.require_tag(id, TagKind::Project)?;
        let days = check_budget(days)?;
        self.conn.execute(
            "UPDATE tags SET budget_days = ?2, updated_at = MAX(?3, updated_at + 1)
             WHERE id = ?1 AND budget_days IS NOT ?2",
            params![id, days, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Müşterinin sözleşme bütçesi (adam-gün; `None` kaldırır).
    pub fn set_client_budget(&self, id: &str, days: Option<f64>) -> Result<()> {
        let days = check_budget(days)?;
        let n = self.conn.execute(
            "UPDATE clients SET budget_days = ?2, updated_at = MAX(?3, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NULL AND budget_days IS NOT ?2",
            params![id, days, ms(Utc::now())],
        )?;
        if n == 0 && !self.clients()?.iter().any(|c| c.id == id) {
            return Err(StoreError::Invalid(format!("müşteri bulunamadı: {id}")));
        }
        Ok(())
    }

    /// Müşteri başına bütçe (adam-gün).
    pub fn client_budgets(&self) -> Result<HashMap<String, f64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, budget_days FROM clients WHERE deleted_at IS NULL AND budget_days > 0",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Bütçeler ve bugüne (`now`) kadar harcanan süre. Adam-gün zaman çizelgesindeki gün
    /// saatiyle çevrilir. Bütçe yoksa geçmiş taranmaz.
    pub fn budgets(&self, now: DateTime<Utc>) -> Result<crate::budget::Budgets> {
        let mut projects: Vec<(String, f64)> = self
            .tag_extras()?
            .into_iter()
            .filter_map(|(id, e)| Some((id, e.budget_days.filter(|d| *d > 0.0)?)))
            .collect();
        let kinds: HashMap<String, TagKind> =
            self.tags()?.into_iter().map(|t| (t.id, t.kind)).collect();
        projects.retain(|(id, _)| kinds.get(id) == Some(&TagKind::Project));
        projects.sort_by(|a, b| a.0.cmp(&b.0));
        let mut clients: Vec<(String, f64)> = self.client_budgets()?.into_iter().collect();
        clients.sort_by(|a, b| a.0.cmp(&b.0));
        let used = if projects.is_empty() && clients.is_empty() {
            HashMap::new()
        } else {
            self.project_totals(DateTime::<Utc>::UNIX_EPOCH, now)?
        };
        Ok(crate::budget::budgets(
            self.timesheet_config()?.day_hours,
            &projects,
            &clients,
            &self.project_clients()?,
            &used,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::MIGRATIONS;
    use super::*;
    use crate::model::Session;
    use chrono::TimeZone;
    use rusqlite::Connection;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn session(title: &str, start: i64, end: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: "com.test.code".into(),
            app_name: "Code".into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(start),
            ended_at: t(end),
            category_id: None,
            project_id: None,
        }
    }

    fn project_seconds(store: &Store, id: &str, from: i64, to: i64) -> i64 {
        store
            .project_totals(t(from), t(to))
            .unwrap()
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    #[test]
    fn archive_columns_migrate_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        let before = MIGRATIONS.len() - 1;
        for sql in &MIGRATIONS[..before] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", before as i64)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO tags (id, kind, name, color, updated_at) VALUES ('p', 'project', 'Eski', 1, 5);
             INSERT INTO clients (id, name, updated_at) VALUES ('c', 'Togg', 5);",
        )
        .unwrap();
        let store = Store::init(conn).unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        assert!(store.tags().unwrap().iter().any(|t| t.name == "Eski"));
        assert!(store.tag_extras().unwrap().is_empty());
        assert!(store.client_budgets().unwrap().is_empty());
        store.set_project_budget("p", Some(3.0)).unwrap();
        store.set_client_budget("c", Some(9.0)).unwrap();
        assert_eq!(store.tag_extras().unwrap()["p"].budget_days, Some(3.0));
        assert_eq!(store.client_budgets().unwrap()["c"], 9.0);
    }

    #[test]
    fn archiving_keeps_history_and_stops_new_classification() {
        let store = Store::open_in_memory().unwrap();
        let kum = store.accept_project_suggestion("Kum").unwrap();
        let old = session("Kum — main.rs", 0, 600);
        store.upsert_session(&old).unwrap();
        // Elle "projesiz" denmiş oturum dondurulurken projeye yazılmaz.
        let mut none = session("Kum — notlar", 600, 700);
        none.project_id = Some(crate::classify::NO_PROJECT.into());
        store.upsert_session(&none).unwrap();
        store
            .set_project_between(t(600), t(700), Some(crate::classify::NO_PROJECT))
            .unwrap();
        assert_eq!(project_seconds(&store, &kum.id, 0, 3600), 600);

        let snap = store.archive_project(&kum.id).unwrap();
        assert!(store.archived_projects().unwrap().contains(&kum.id));
        assert!(store.rules().unwrap().iter().all(|r| r.tag_id != kum.id));
        // Geçmiş aynı kalır; arşivden sonraki oturum projeye düşmez.
        store
            .upsert_session(&session("Kum — lib.rs", 1000, 1300))
            .unwrap();
        assert_eq!(project_seconds(&store, &kum.id, 0, 3600), 600);
        let stamped = store.sessions_between(t(0), t(600)).unwrap();
        assert_eq!(stamped[0].project_id.as_deref(), Some(kum.id.as_str()));
        assert_eq!(
            store.sessions_between(t(600), t(700)).unwrap()[0]
                .project_id
                .as_deref(),
            Some(crate::classify::NO_PROJECT)
        );
        // Arşivlenen iş yeniden proje diye önerilmez.
        let suggested = store.suggestions(t(3600)).unwrap();
        assert!(suggested.projects.iter().all(|p| p.name != "Kum"));

        // Arşivden çıkınca kurallar yeniden işler.
        let prev = store.unarchive_project(&kum.id).unwrap();
        assert!(prev.is_some());
        assert_eq!(project_seconds(&store, &kum.id, 0, 3600), 900);

        // Geri alma: arşiv anı ve dondurulan oturumlar eski haline döner.
        store.set_tag_archived_at(&kum.id, prev).unwrap();
        assert_eq!(project_seconds(&store, &kum.id, 0, 3600), 600);
        store.restore_snapshot(&snap).unwrap();
        store.set_tag_archived_at(&kum.id, None).unwrap();
        assert_eq!(
            store.sessions_between(t(0), t(600)).unwrap()[0].project_id,
            None
        );
        assert_eq!(project_seconds(&store, &kum.id, 0, 3600), 900);
    }

    #[test]
    fn only_projects_can_be_archived_or_budgeted() {
        let store = Store::open_in_memory().unwrap();
        let category = store.tags().unwrap()[0].id.clone();
        assert!(store.archive_project(&category).is_err());
        assert!(store.set_project_budget(&category, Some(1.0)).is_err());
        let kum = store.accept_project_suggestion("Kum").unwrap();
        for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert!(store.set_project_budget(&kum.id, Some(bad)).is_err());
        }
        assert!(store.set_client_budget("yok", Some(1.0)).is_err());
    }

    #[test]
    fn budgets_count_all_time_project_time_in_man_days() {
        let store = Store::open_in_memory().unwrap();
        // Bütçe yokken boş (geçmiş taranmaz).
        assert_eq!(store.budgets(t(0)).unwrap().projects, []);
        let kum = store.accept_project_suggestion("Kum").unwrap();
        let other = store.accept_project_suggestion("Trumore").unwrap();
        let togg = Client {
            id: Uuid::new_v4().to_string(),
            name: "Togg".into(),
        };
        store.upsert_client(&togg, 0).unwrap();
        store.set_project_client(&kum.id, Some(&togg.id)).unwrap();
        store.set_project_client(&other.id, Some(&togg.id)).unwrap();
        let mut config = store.timesheet_config().unwrap();
        config.day_hours = 6.0;
        store.save_timesheet_config(&config).unwrap();
        store
            .upsert_session(&session("Kum — a", 0, 3 * 3600))
            .unwrap();
        store
            .upsert_session(&session("Trumore — b", 4 * 3600, 7 * 3600))
            .unwrap();
        store.set_project_budget(&kum.id, Some(2.0)).unwrap();
        store.set_client_budget(&togg.id, Some(1.0)).unwrap();
        // Arşivdeki projenin süresi de bütçeye sayılır.
        store.archive_project(&kum.id).unwrap();

        let b = store.budgets(t(8 * 3600)).unwrap();
        assert_eq!(b.day_hours, 6.0);
        assert_eq!(b.projects.len(), 1);
        assert_eq!(b.projects[0].budget_seconds, 12 * 3600);
        assert_eq!(b.projects[0].used_seconds, 3 * 3600);
        assert_eq!(b.clients[0].budget_seconds, 6 * 3600);
        assert_eq!(b.clients[0].used_seconds, 6 * 3600);
        assert_eq!(
            b.clients[0].level(),
            Some(crate::budget::BudgetLevel::Reached)
        );
    }
}
