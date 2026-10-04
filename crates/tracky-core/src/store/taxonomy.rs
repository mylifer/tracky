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
        let tx = self.conn.unchecked_transaction()?;
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

    pub fn rules(&self) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tag_id, field, pattern FROM rules
             WHERE deleted_at IS NULL ORDER BY position, rowid",
        )?;
        let rows = stmt.query_map([], |r| {
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
        Ok(suggest::suggest(
            &sessions,
            &self.tags()?,
            &self.rules()?,
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
