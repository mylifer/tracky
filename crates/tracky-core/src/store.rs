use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::classify::{
    Classifier, Client, DEFAULT_CATEGORIES, Rule, RuleField, Tag, TagKind, default_id,
};
use crate::model::{FocusTimer, MANUAL_APP_ID, Session};
use crate::privacy::PrivacySettings;
use crate::report::{self, Report};
use crate::suggest::{self, Suggestions};
use crate::timesheet::{self, EntryKind, Meeting, MeetingProject, TimesheetConfig, TimesheetEntry};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("veritabanı hatası: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("ayar okunamadı: {0}")]
    Json(#[from] serde_json::Error),
    #[error("geçersiz kayıt: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Sırasıyla uygulanan şema göçleri; indeks + 1 = `PRAGMA user_version`.
///
/// Senkronizasyona hazırlık: tüm satırlar UUID ile tanımlanır, her satırda
/// `device_id`, `updated_at` (son değişiklik) ve `deleted_at` (yumuşak silme)
/// bulunur; `synced_at` yalnızca yerelde tutulur.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,
    device_id    TEXT NOT NULL,
    app_id       TEXT NOT NULL,
    app_name     TEXT NOT NULL,
    title        TEXT NOT NULL,
    url          TEXT,
    domain       TEXT,
    started_at   INTEGER NOT NULL,  -- unix ms, UTC
    ended_at     INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    deleted_at   INTEGER,
    synced_at    INTEGER,
    CHECK (ended_at >= started_at)
);
CREATE INDEX sessions_started_at ON sessions (started_at);
CREATE INDEX sessions_ended_at   ON sessions (ended_at);
CREATE INDEX sessions_unsynced   ON sessions (updated_at) WHERE synced_at IS NULL OR synced_at < updated_at;
"#,
    r#"
-- Uygulama ayarları; değer JSON. Cihazlar arası senkronize edilebilir.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
"#,
    r#"
-- Kategoriler ve projeler (kind) ile oturumları onlara bağlayan kurallar.
CREATE TABLE tags (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('category', 'project')),
    name       TEXT NOT NULL,
    color      INTEGER NOT NULL CHECK (color BETWEEN 1 AND 8),
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    synced_at  INTEGER
);
CREATE TABLE rules (
    id         TEXT PRIMARY KEY,
    tag_id     TEXT NOT NULL REFERENCES tags (id),
    field      TEXT NOT NULL CHECK (field IN ('app', 'title')),
    pattern    TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    synced_at  INTEGER
);
CREATE INDEX rules_tag ON rules (tag_id);
"#,
    r#"
-- Oturuma elle verilen kategori (takvimde bloğu atama, manuel kayıt).
ALTER TABLE sessions ADD COLUMN category_id TEXT;
"#,
    r#"
-- Odak zamanlayıcıları (yalnızca bu cihazda; senkronize edilmez).
CREATE TABLE focus_timers (
    id          TEXT PRIMARY KEY,
    started_at  INTEGER NOT NULL,
    planned_end INTEGER NOT NULL,
    ended_at    INTEGER,
    CHECK (planned_end > started_at)
);
CREATE INDEX focus_timers_start ON focus_timers (started_at);
"#,
    r#"
-- En uzun oturumun süresi (ms). Aralık sorguları "başlangıç - en uzun süre" alt sınırıyla
-- started_at indeksini iki yönden kullanır; yoksa son haftanın raporu tüm geçmişi tarar.
-- Tetikleyiciler her yazma yolunu (takip, düzenleme, eşitleme) kapsar; değer yalnızca büyür.
CREATE TABLE session_stats (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    max_duration INTEGER NOT NULL
);
INSERT INTO session_stats VALUES (1, (SELECT COALESCE(MAX(ended_at - started_at), 0) FROM sessions));
CREATE TRIGGER sessions_max_duration_insert AFTER INSERT ON sessions BEGIN
    UPDATE session_stats SET max_duration = NEW.ended_at - NEW.started_at
    WHERE id = 1 AND max_duration < NEW.ended_at - NEW.started_at;
END;
CREATE TRIGGER sessions_max_duration_update AFTER UPDATE OF started_at, ended_at ON sessions BEGIN
    UPDATE session_stats SET max_duration = NEW.ended_at - NEW.started_at
    WHERE id = 1 AND max_duration < NEW.ended_at - NEW.started_at;
END;
"#,
    r#"
-- Uygulama başına son kullanım: bilinen uygulamalar listesi tabloyu taramadan,
-- uygulama başına tek indeks aramasıyla çıkar; uygulamanın başlık dökümü de hızlanır.
CREATE INDEX sessions_app_ended ON sessions (app_id, ended_at);
"#,
    r#"
-- Oturuma elle verilen proje (takvimde bloğu ya da aralığı projeye atama, elle kayıt).
ALTER TABLE sessions ADD COLUMN project_id TEXT;
"#,
    r#"
-- Zaman çizelgesi: onaylanmış iş kayıtları (yalnızca bu cihazda; senkronize edilmez).
-- Excel'e aktarılınca exported_at dolar ve kayıt bir daha gönderilmez.
CREATE TABLE timesheet_entries (
    id          TEXT PRIMARY KEY,
    date        TEXT NOT NULL,     -- YYYY-MM-DD (yerel)
    start       TEXT NOT NULL,     -- HH:MM (yerel)
    hours       REAL NOT NULL CHECK (hours > 0),
    kind        TEXT NOT NULL CHECK (kind IN ('Working', 'Online', 'F2F')),
    details     TEXT NOT NULL,
    party       TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    division    TEXT NOT NULL,
    exported_at INTEGER,
    created_at  INTEGER NOT NULL
);
CREATE INDEX timesheet_entries_date ON timesheet_entries (date);
"#,
    r#"
-- Zaman çizelgesi: takip edilen gerçek süre (saat); "hours" çeyrek saate yuvarlanmış olandır.
ALTER TABLE timesheet_entries ADD COLUMN actual_hours REAL;
"#,
    r#"
-- Müşteriler; projeler bir müşteriye bağlanabilir (tags.client_id, yalnızca projelerde).
CREATE TABLE clients (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    synced_at  INTEGER
);
ALTER TABLE tags ADD COLUMN client_id TEXT;
"#,
];

/// Onaylanmış zaman çizelgesi kaydı.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEntry {
    pub id: String,
    #[serde(flatten)]
    pub entry: TimesheetEntry,
    /// Excel'e aktarıldığı an; doluysa kayıt değiştirilemez.
    pub exported_at: Option<DateTime<Utc>>,
}

fn parse_kind(s: &str) -> Option<EntryKind> {
    match s {
        "Working" => Some(EntryKind::Working),
        "Online" => Some(EntryKind::Online),
        "F2F" => Some(EntryKind::F2F),
        _ => None,
    }
}

/// `[?1, ?2)` ile kesişen oturumlar. Üçüncü koşul sonucu değiştirmez (kesişen her oturum
/// en uzun oturumdan kısadır), yalnızca indeksin alt sınırıdır.
const OVERLAPS: &str = "started_at < ?2 AND ended_at > ?1
    AND started_at >= ?1 - (SELECT max_duration FROM session_stats)";

const DEFAULTS_SEEDED_KEY: &str = "default_tags_seeded";

const PRIVACY_KEY: &str = "privacy";
/// Zaman çizelgesi ayarları.
const TIMESHEET_KEY: &str = "timesheet";
/// Takvim toplantı serilerinin elle verilen projesi (UID → proje; `null`: yoksay).
const MEETING_ASSIGNMENTS_KEY: &str = "meeting_assignments";
/// Yoksayılan öneri anahtarları.
const DISMISSED_SUGGESTIONS_KEY: &str = "dismissed_suggestions";
/// Öneriler bu kadar günlük geçmişe bakar.
const SUGGEST_DAYS: i64 = 14;

/// Bir zaman aralığında bir anahtar (uygulama, domain...) için toplam süre.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UsageTotal {
    pub key: String,
    pub label: String,
    pub seconds: i64,
}

/// (Projesi belli toplantılar ve projeleri, hiçbir projeye düşmeyen toplantılar).
pub type SplitMeetings = (Vec<(Meeting, String)>, Vec<Meeting>);

pub struct Store {
    conn: Connection,
    device_id: Uuid,
    /// Bu açılışa özgü, diske yazılmayan kimlik (eşitlemede satırı yazanı tanımak için).
    /// Kopyalanan bir veritabanı (örn. Taşıma Yardımcısı) cihaz kimliğini de taşır;
    /// kalıcı kimlik kullanılsaydı iki kurulum birbirinin satırlarını hiç çekmezdi.
    instance_id: Uuid,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // WAL'da önerilen ayar: çökmede veri kaybı ve bozulma olmaz, yalnızca elektrik
        // kesintisinde son birkaç saniye gidebilir. Takip birkaç saniyede bir yazdığı için
        // FULL her yazmada diske zorlama (fsync) yapıp pili ve diski boşuna yorardı.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        let device_id = device_id(&conn)?;
        let store = Self {
            conn,
            device_id,
            instance_id: Uuid::new_v4(),
        };
        store.seed_default_tags()?;
        Ok(store)
    }

    /// Senkronizasyon modülü için ham bağlantı.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Bu kurulumun kalıcı kimliği; senkronizasyonda kayıtların kaynağını belirtir.
    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    /// Bu açılışa özgü kimlik; her açılışta yenidir.
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    /// Kayıtlı gizlilik ayarları; hiç kaydedilmediyse varsayılanlar.
    pub fn privacy_settings(&self) -> Result<PrivacySettings> {
        Ok(self.setting(PRIVACY_KEY)?.unwrap_or_default())
    }

    pub fn save_privacy_settings(&self, settings: &PrivacySettings) -> Result<()> {
        self.save_setting(PRIVACY_KEY, settings)
    }

    /// JSON olarak saklanan bir ayarı okur; yoksa `None`.
    pub fn setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(raw.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub fn save_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, serde_json::to_string(value)?, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Oturumu ekler ya da (devam eden oturumun periyodik kaydında) günceller.
    pub fn upsert_session(&self, s: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
                (id, device_id, app_id, app_name, title, url, domain, started_at, ended_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET
                title = excluded.title,
                url = excluded.url,
                domain = excluded.domain,
                -- Süren oturum silindiyse (takvimde blok silme, başka cihaz) takip
                -- sürüyor demektir: kayıt silinme anından itibaren yeniden başlar.
                started_at = CASE WHEN sessions.deleted_at IS NULL THEN sessions.started_at
                    ELSE MAX(sessions.started_at, MIN(sessions.deleted_at, excluded.ended_at)) END,
                deleted_at = NULL,
                ended_at = excluded.ended_at,
                updated_at = MAX(excluded.updated_at, sessions.updated_at + 1)",
            params![
                s.id.to_string(),
                self.device_id.to_string(),
                s.app_id,
                s.app_name,
                s.title,
                s.url,
                s.domain,
                ms(s.started_at),
                ms(s.ended_at),
                ms(Utc::now()),
            ],
        )?;
        Ok(())
    }

    /// `[from, to)` ile kesişen oturumlar, başlangıca göre sıralı.
    pub fn sessions_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, app_id, app_name, title, url, domain, started_at, ended_at, category_id,
                    project_id
             FROM sessions
             WHERE deleted_at IS NULL AND {OVERLAPS}
             ORDER BY started_at"
        ))?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Session {
                    id: Uuid::nil(),
                    app_id: r.get(1)?,
                    app_name: r.get(2)?,
                    title: r.get(3)?,
                    url: r.get(4)?,
                    domain: r.get(5)?,
                    started_at: from_ms(r.get(6)?),
                    ended_at: from_ms(r.get(7)?),
                    category_id: r.get(8)?,
                    project_id: r.get(9)?,
                },
            ))
        })?;
        rows.map(|row| {
            let (id, mut s) = row?;
            s.id = Uuid::parse_str(&id).map_err(|e| StoreError::Invalid(e.to_string()))?;
            Ok(s)
        })
        .collect()
    }

    /// Uygulama başına toplam süre; aralık dışına taşan kısımlar kırpılır.
    pub fn app_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals("app_id", "app_name", from, to)
    }

    /// Domain başına toplam süre (yalnızca URL'si bilinen oturumlar).
    pub fn domain_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals("domain", "domain", from, to)
    }

    /// İlk açılışta varsayılan kategorileri ekler (kullanıcı silerse geri gelmez).
    fn seed_default_tags(&self) -> Result<()> {
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

    /// Oturumu yumuşak siler (örn. boşta kalma sonrası geçersiz kalan kayıt).
    pub fn delete_session(&self, id: &Uuid) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "UPDATE sessions SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id.to_string(), now],
        )?;
        Ok(())
    }

    /// `[from, to)` ile kesişen oturumlara elle kategori verir (`None`: kurallara
    /// dön). Takvimdeki bir blok bu oturumlardan oluşur. Değişen satır sayısı.
    pub fn set_category_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
    ) -> Result<usize> {
        if let Some(id) = category_id {
            self.require_tag(id, TagKind::Category)?;
        }
        let tx = self.conn.unchecked_transaction()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET category_id = ?3, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND category_id IS NOT ?3"
            ),
            params![ms(from), ms(to), category_id, ms(Utc::now())],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` aralığındaki oturumlara elle proje verir (sınırda bölünür);
    /// `None` kurallara döndürür, [`crate::classify::NO_PROJECT`] projesiz yapar. Değişen
    /// satır sayısı.
    pub fn set_project_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        if let Some(id) = project_id.filter(|id| *id != crate::classify::NO_PROJECT) {
            self.require_tag(id, TagKind::Project)?;
        }
        let tx = self.conn.unchecked_transaction()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET project_id = ?3, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND project_id IS NOT ?3"
            ),
            params![ms(from), ms(to), project_id, ms(Utc::now())],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` içindeki süreyi yumuşak siler; sınırı aşan oturumların dışarıda
    /// kalan kısmı korunur. Silinen satır sayısı.
    pub fn delete_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET deleted_at = ?3, updated_at = MAX(?3, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}"
            ),
            params![ms(from), ms(to), ms(Utc::now())],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` sınırını aşan oturumları sınırlarda böler; sonra aralıkla
    /// kesişen her oturum tamamen aralığın içindedir. Asıl kimlik en son parçada
    /// kalır: süren oturumu takip eden motor doğru satırı uzatmaya devam eder.
    fn split_at(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        let (from, to, now) = (ms(from), ms(to), ms(Utc::now()));
        let partial: Vec<(String, i64, i64)> = self
            .conn
            .prepare(&format!(
                "SELECT id, started_at, ended_at FROM sessions
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND (started_at < ?1 OR ended_at > ?2)"
            ))?
            .query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, start, end) in partial {
            let mut parts = Vec::new();
            if start < from {
                parts.push((start, from));
            }
            parts.push((start.max(from), end.min(to)));
            if end > to {
                parts.push((to, end));
            }
            let (keep_start, keep_end) = parts.pop().expect("en az bir parça");
            for (a, b) in parts {
                self.conn.execute(
                    "INSERT INTO sessions (id, device_id, app_id, app_name, title, url, domain,
                         category_id, project_id, started_at, ended_at, updated_at)
                     SELECT ?2, device_id, app_id, app_name, title, url, domain, category_id,
                         project_id, ?3, ?4, ?5
                     FROM sessions WHERE id = ?1",
                    params![id, Uuid::new_v4().to_string(), a, b, now],
                )?;
            }
            self.conn.execute(
                "UPDATE sessions SET started_at = ?2, ended_at = ?3, updated_at = MAX(?4, updated_at + 1)
                 WHERE id = ?1",
                params![id, keep_start, keep_end, now],
            )?;
        }
        Ok(())
    }

    /// Elle kayıt ekler (bilgisayar dışında geçen toplantı, okuma...).
    pub fn add_manual_session(
        &self,
        label: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Session> {
        let label = label.trim();
        if label.is_empty() {
            return Err(StoreError::Invalid("kayıt adı boş".into()));
        }
        if to <= from {
            return Err(StoreError::Invalid(
                "bitiş başlangıçtan sonra olmalı".into(),
            ));
        }
        if let Some(id) = category_id {
            self.require_tag(id, TagKind::Category)?;
        }
        if let Some(id) = project_id {
            self.require_tag(id, TagKind::Project)?;
        }
        // Takip edilen süreyle çakışırsa aynı dakikalar iki kez sayılırdı.
        if !self.sessions_between(from, to)?.is_empty() {
            return Err(StoreError::Invalid(
                "bu aralıkta zaten kayıt var; önce o bloğu silin".into(),
            ));
        }
        let session = Session {
            id: Uuid::new_v4(),
            app_id: format!("{MANUAL_APP_ID}/{label}"),
            app_name: label.into(),
            title: label.into(),
            url: None,
            domain: None,
            started_at: from,
            ended_at: to,
            category_id: category_id.map(Into::into),
            project_id: project_id.map(Into::into),
        };
        self.upsert_session(&session)?;
        self.conn.execute(
            "UPDATE sessions SET category_id = ?2, project_id = ?3 WHERE id = ?1",
            params![
                session.id.to_string(),
                session.category_id,
                session.project_id
            ],
        )?;
        Ok(session)
    }

    fn require_tag(&self, id: &str, kind: TagKind) -> Result<()> {
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

    /// Senkronizasyon başka hesaba/projeye bağlandığında: imleçleri sil, her şeyi
    /// yeniden gönderilecek işaretle.
    pub fn reset_sync_state(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM settings WHERE key LIKE 'sync_cursor:%';
             UPDATE sessions SET synced_at = NULL;
             UPDATE tags SET synced_at = NULL;
             UPDATE rules SET synced_at = NULL;
             UPDATE clients SET synced_at = NULL;",
        )?;
        Ok(())
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

    /// Yumuşak siler; kuralları da birlikte silinir.
    pub fn delete_tag(&self, id: &str) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "UPDATE tags SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1) WHERE id = ?1",
            params![id, now],
        )?;
        self.conn.execute(
            "UPDATE rules SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
             WHERE tag_id = ?1 AND deleted_at IS NULL",
            params![id, now],
        )?;
        Ok(())
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
        rows.map(|row| {
            let (id, tag_id, field, pattern) = row?;
            let field = RuleField::parse(&field)
                .ok_or_else(|| StoreError::Invalid(format!("bilinmeyen kural alanı: {field}")))?;
            Ok(Rule {
                id,
                tag_id,
                field,
                pattern,
            })
        })
        .collect()
    }

    pub fn upsert_rule(&self, rule: &Rule) -> Result<()> {
        let pattern = rule.pattern.trim();
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
        for rule in self.rules()? {
            if rule.field == RuleField::App && is_category(&rule.tag_id) && rule.matches(app_id, "")
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

    /// `[from, to)` aralığında başlığında ya da uygulama adında `query` geçen süre.
    pub fn search(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
    ) -> Result<crate::search::SearchResult> {
        let sessions = self.sessions_between(from, to)?;
        Ok(crate::search::search(
            &sessions, query, from, to, day_starts,
        ))
    }

    /// Aramayla eşleşen oturumların CSV dökümü; oturumlar aralığa kırpılır.
    pub fn export_search_csv(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<String> {
        let needle = crate::search::fold(query.trim());
        let sessions: Vec<Session> = self
            .sessions_between(from, to)?
            .into_iter()
            .filter(|s| !needle.is_empty() && crate::search::matches(s, &needle))
            .map(|mut s| {
                s.started_at = s.started_at.max(from);
                s.ended_at = s.ended_at.min(to);
                s
            })
            .filter(|s| s.ended_at > s.started_at)
            .collect();
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(crate::export::sessions_csv(&sessions, &tags, &classifier))
    }

    /// Ardışık dönemlerde proje ve kategori süreleri; `bounds` dönem sınırları (n + 1 öğe).
    pub fn trends(&self, bounds: &[DateTime<Utc>]) -> Result<crate::trends::Trends> {
        let (Some(first), Some(last)) = (bounds.first(), bounds.last()) else {
            return Ok(Default::default());
        };
        let sessions = self.sessions_between(*first, *last)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        Ok(crate::trends::trends(&sessions, &classifier, bounds))
    }

    pub fn timesheet_config(&self) -> Result<TimesheetConfig> {
        Ok(self.setting(TIMESHEET_KEY)?.unwrap_or_default())
    }

    pub fn save_timesheet_config(&self, config: &TimesheetConfig) -> Result<()> {
        self.save_setting(TIMESHEET_KEY, config)
    }

    /// Toplantı serilerinin elle verilen projeleri (UID → proje; `None`: yoksayıldı).
    pub fn meeting_assignments(&self) -> Result<HashMap<String, Option<String>>> {
        Ok(self.setting(MEETING_ASSIGNMENTS_KEY)?.unwrap_or_default())
    }

    /// Toplantı serisini projeye atar (`None`: zaman çizelgesine alma).
    pub fn assign_meeting(&self, uid: &str, project: Option<&str>) -> Result<()> {
        let mut all = self.meeting_assignments()?;
        all.insert(uid.to_string(), project.map(str::to_string));
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)
    }

    /// Yoksayılan toplantıları geri getirir; sayısını döndürür.
    pub fn restore_ignored_meetings(&self) -> Result<usize> {
        let mut all = self.meeting_assignments()?;
        let before = all.len();
        all.retain(|_, p| p.is_some());
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)?;
        Ok(before - all.len())
    }

    /// Toplantıları projesine göre ayırır: (projesi belli olanlar, hiçbir projeye düşmeyenler).
    pub fn classify_meetings(&self, meetings: &[Meeting]) -> Result<SplitMeetings> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let assigned = self.meeting_assignments()?;
        let (mut known, mut unassigned) = (Vec::new(), Vec::new());
        for m in meetings {
            match timesheet::meeting_project(m, &classifier, &assigned) {
                MeetingProject::Project(p) => known.push((m.clone(), p)),
                MeetingProject::Unassigned => unassigned.push(m.clone()),
                MeetingProject::Ignored => {}
            }
        }
        Ok((known, unassigned))
    }

    /// Günün (`day_start`–`day_end`, yerel gün) oturumlarından ve takvim toplantılarından
    /// iş kaydı önerileri.
    pub fn propose_timesheet(
        &self,
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        self.propose_from(
            &self.sessions_between(day_start, day_end)?,
            day_start,
            day_end,
            meetings,
        )
    }

    /// Yalnızca toplantılardan iş kaydı önerileri (takip edilen süre olmadan).
    pub fn propose_meetings(
        &self,
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        self.propose_from(&[], day_start, day_end, meetings)
    }

    fn propose_from(
        &self,
        sessions: &[Session],
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        let names = tags
            .iter()
            .filter(|t| t.kind == TagKind::Project)
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();
        let (meetings, _) = self.classify_meetings(meetings)?;
        Ok(timesheet::propose(
            sessions,
            &meetings,
            &classifier,
            &names,
            &self.timesheet_config()?,
            day_start,
            day_end,
        ))
    }

    /// `[from, to]` tarihleri (dahil) arasındaki onaylanmış kayıtlar, tarih ve saate göre.
    pub fn timesheet_entries(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<SavedEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, date, start, hours, kind, details, party, project_id, division, exported_at,
                    actual_hours
             FROM timesheet_entries WHERE date >= ?1 AND date <= ?2 ORDER BY date, start",
        )?;
        let rows = stmt.query_map(params![from.to_string(), to.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<f64>>(10)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                date,
                start,
                hours,
                kind,
                details,
                party,
                project_id,
                division,
                exported,
                actual,
            ) = row?;
            let bad = |what: &str| StoreError::Invalid(format!("zaman çizelgesi {what}: {id}"));
            Ok(SavedEntry {
                entry: TimesheetEntry {
                    date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                        .map_err(|_| bad("tarihi"))?,
                    start: NaiveTime::parse_from_str(&start, "%H:%M").map_err(|_| bad("saati"))?,
                    hours,
                    actual_hours: actual,
                    kind: parse_kind(&kind).ok_or_else(|| bad("türü"))?,
                    details,
                    party,
                    project_id,
                    division,
                },
                exported_at: exported.map(from_ms),
                id,
            })
        })
        .collect()
    }

    /// Günün aktarılmamış kayıtlarını verilenlerle değiştirir (onaylama, yeniden öneri).
    /// Excel'e aktarılmış kayıtlara dokunulmaz ve aktarılan iş yeniden eklenmez
    /// ([`timesheet::without_exported`]); yoksa bir sonraki aktarımda dosyaya iki kez yazılırdı.
    pub fn replace_timesheet_day(&self, date: NaiveDate, entries: &[TimesheetEntry]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "DELETE FROM timesheet_entries WHERE date = ?1 AND exported_at IS NULL",
            [date.to_string()],
        )?;
        let exported: Vec<TimesheetEntry> = self
            .timesheet_entries(date, date)?
            .into_iter()
            .map(|e| e.entry)
            .collect();
        for e in timesheet::without_exported(entries, &exported) {
            self.insert_entry(&Uuid::new_v4().to_string(), &e)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Kaydı ekler ya da (aynı kimlikle) günceller; aktarılmış kayıt değiştirilemez.
    pub fn save_timesheet_entry(&self, id: Option<&str>, entry: &TimesheetEntry) -> Result<String> {
        if !(entry.hours > 0.0 && entry.hours <= 24.0) {
            return Err(StoreError::Invalid("saat 0 ile 24 arasında olmalı".into()));
        }
        let id = id.map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
        let exported: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT exported_at FROM timesheet_entries WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        if matches!(exported, Some(Some(_))) {
            return Err(StoreError::Invalid(
                "Excel'e aktarılmış kayıt değiştirilemez".into(),
            ));
        }
        self.conn
            .execute("DELETE FROM timesheet_entries WHERE id = ?1", [&id])?;
        self.insert_entry(&id, entry)?;
        Ok(id)
    }

    pub fn delete_timesheet_entry(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM timesheet_entries WHERE id = ?1 AND exported_at IS NULL",
            [id],
        )?;
        Ok(())
    }

    /// Excel'e aktarılan kayıtları işaretler.
    pub fn mark_timesheet_exported(&self, ids: &[String], at: DateTime<Utc>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for id in ids {
            self.conn.execute(
                "UPDATE timesheet_entries SET exported_at = ?2 WHERE id = ?1",
                params![id, ms(at)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn insert_entry(&self, id: &str, e: &TimesheetEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO timesheet_entries
                (id, date, start, hours, kind, details, party, project_id, division, created_at,
                 actual_hours)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                e.date.to_string(),
                e.start.format("%H:%M").to_string(),
                e.hours,
                e.kind.label(),
                e.details.trim(),
                e.party.trim(),
                e.project_id,
                e.division.trim(),
                ms(Utc::now()),
                e.actual_hours,
            ],
        )?;
        Ok(())
    }

    /// Son iki haftanın oturumlarından proje ve kategori önerileri.
    pub fn suggestions(&self, now: DateTime<Utc>) -> Result<Suggestions> {
        let sessions = self.sessions_between(now - chrono::Duration::days(SUGGEST_DAYS), now)?;
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
            RuleField::Title => self.upsert_rule(&Rule {
                id: Uuid::new_v4().to_string(),
                tag_id: category_id.to_string(),
                field,
                pattern: pattern.to_string(),
            }),
        }
    }

    /// Tüm oturumların CSV dökümü.
    pub fn export_csv(&self) -> Result<String> {
        let sessions = self.sessions_between(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC)?;
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(crate::export::sessions_csv(&sessions, &tags, &classifier))
    }

    /// `[from, to)` raporu; `day_starts` yerel gün sınırlarıdır.
    pub fn report(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        with_timeline: bool,
    ) -> Result<Report> {
        let sessions = self.sessions_between(from, to)?;
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        let mut report = report::build(
            &sessions,
            &tags,
            &classifier,
            from,
            to,
            day_starts,
            with_timeline,
        );
        if with_timeline {
            report.focus_timers = self.focus_timers_between(from, to)?;
        }
        Ok(report)
    }

    /// Yeni odak zamanlayıcısı başlatır; süren varsa şimdi bitirilir.
    pub fn start_focus(&self, minutes: u32, now: DateTime<Utc>) -> Result<FocusTimer> {
        if !(1..=8 * 60).contains(&minutes) {
            return Err(StoreError::Invalid(format!(
                "odak süresi geçersiz: {minutes}"
            )));
        }
        self.stop_focus(now)?;
        let timer = FocusTimer {
            id: Uuid::new_v4().to_string(),
            start: now,
            planned_end: now + chrono::Duration::minutes(i64::from(minutes)),
            end: None,
        };
        self.conn.execute(
            "INSERT INTO focus_timers (id, started_at, planned_end) VALUES (?1, ?2, ?3)",
            params![timer.id, ms(timer.start), ms(timer.planned_end)],
        )?;
        Ok(timer)
    }

    /// Süren zamanlayıcıyı erken bitirir (kısa denemeler kayıt bırakmaz).
    pub fn stop_focus(&self, now: DateTime<Utc>) -> Result<Option<FocusTimer>> {
        let Some(mut timer) = self.active_focus()? else {
            return Ok(None);
        };
        if now - timer.start < chrono::Duration::minutes(1) {
            self.conn
                .execute("DELETE FROM focus_timers WHERE id = ?1", [&timer.id])?;
            return Ok(None);
        }
        let end = now.min(timer.planned_end);
        self.conn.execute(
            "UPDATE focus_timers SET ended_at = ?2 WHERE id = ?1",
            params![timer.id, ms(end)],
        )?;
        timer.end = Some(end);
        Ok(Some(timer))
    }

    /// Süresi dolmuş zamanlayıcıyı planlanan bitişte kapatır ve döndürür
    /// (bildirim için). Uygulama kapalıyken dolduysa da kapatılır.
    pub fn complete_due_focus(&self, now: DateTime<Utc>) -> Result<Option<FocusTimer>> {
        match self.active_focus()? {
            Some(mut t) if t.planned_end <= now => {
                self.conn.execute(
                    "UPDATE focus_timers SET ended_at = planned_end WHERE id = ?1",
                    [&t.id],
                )?;
                t.end = Some(t.planned_end);
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    pub fn active_focus(&self) -> Result<Option<FocusTimer>> {
        Ok(self
            .focus_query(
                "WHERE ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
                params![],
            )?
            .pop())
    }

    /// `[from, to)` ile kesişen zamanlayıcılar (sürenler dahil).
    pub fn focus_timers_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<FocusTimer>> {
        self.focus_query(
            "WHERE started_at < ?2 AND COALESCE(ended_at, planned_end) > ?1 ORDER BY started_at",
            params![ms(from), ms(to)],
        )
    }

    fn focus_query(&self, filter: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<FocusTimer>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, started_at, planned_end, ended_at FROM focus_timers {filter}"
        ))?;
        let rows = stmt.query_map(args, |r| {
            Ok(FocusTimer {
                id: r.get(0)?,
                start: from_ms(r.get(1)?),
                planned_end: from_ms(r.get(2)?),
                end: r.get::<_, Option<i64>>(3)?.map(from_ms),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Kategori başına toplam süre (saniye); kategorisiz süre dahil edilmez.
    pub fn category_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        self.tag_totals(from, to, |c| c.category)
    }

    /// Proje başına toplam süre (saniye); projesiz süre dahil edilmez.
    pub fn project_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        self.tag_totals(from, to, |c| c.project)
    }

    fn tag_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        pick: fn(crate::classify::Classification) -> Option<String>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut out = std::collections::HashMap::new();
        for s in self.sessions_between(from, to)? {
            let secs = (s.ended_at.min(to) - s.started_at.max(from)).num_seconds();
            if let (Some(id), true) = (pick(classifier.classify(&s)), secs > 0) {
                *out.entry(id).or_default() += secs;
            }
        }
        Ok(out)
    }

    /// Son kullanılan uygulamalar (kural ve gizlilik seçicileri için), en yeni önce; ad en
    /// son kullanıldığı haliyle. Süre hesaplanmaz (`seconds` 0): tüm geçmişi toplamak her
    /// sayfa açılışında tabloyu tarardı. Uygulamalar (app_id, ended_at) indeksinde atlayarak
    /// bulunur ("loose index scan"): uygulama başına bir arama.
    pub fn known_apps(&self, limit: usize) -> Result<Vec<UsageTotal>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE ids(id) AS (
                 SELECT MIN(app_id) FROM sessions
                 UNION ALL
                 SELECT (SELECT MIN(app_id) FROM sessions WHERE app_id > ids.id)
                 FROM ids WHERE ids.id IS NOT NULL
             ), latest(r) AS (
                 SELECT (SELECT rowid FROM sessions
                         WHERE app_id = ids.id AND deleted_at IS NULL
                         ORDER BY ended_at DESC LIMIT 1)
                 FROM ids WHERE ids.id IS NOT NULL
             )
             SELECT s.app_id, s.app_name, 0
             FROM latest JOIN sessions s ON s.rowid = latest.r
             ORDER BY s.ended_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(UsageTotal {
                key: r.get(0)?,
                label: r.get(1)?,
                seconds: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Bir uygulamanın pencere başlıklarına göre süre dağılımı.
    pub fn title_totals(
        &self,
        app_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageTotal>> {
        let mut stmt = self.conn.prepare(
            "SELECT title, title, SUM(MIN(ended_at, ?3) - MAX(started_at, ?2)) / 1000 AS secs
             FROM sessions
             WHERE deleted_at IS NULL AND app_id = ?1
               AND started_at < ?3 AND ended_at > ?2
               AND started_at >= ?2 - (SELECT max_duration FROM session_stats)
             GROUP BY title
             ORDER BY secs DESC, 1",
        )?;
        let rows = stmt.query_map(params![app_id, ms(from), ms(to)], |r| {
            Ok(UsageTotal {
                key: r.get(0)?,
                label: r.get(1)?,
                seconds: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn totals(
        &self,
        key_col: &str,
        label_col: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageTotal>> {
        // Sütun adları sabit; kullanıcı girdisi değildir.
        let sql = format!(
            "SELECT {key_col}, MAX({label_col}),
                    SUM((MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000) AS secs
             FROM sessions
             WHERE deleted_at IS NULL AND {key_col} IS NOT NULL
               AND {OVERLAPS}
             GROUP BY {key_col}
             ORDER BY secs DESC, 2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok(UsageTotal {
                key: r.get(0)?,
                label: r.get(1)?,
                seconds: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let version = usize::try_from(version).unwrap_or(usize::MAX);
    if version > MIGRATIONS.len() {
        return Err(StoreError::Invalid(format!(
            "veritabanı şema sürümü ({version}) bu uygulamadan yeni"
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn device_id(conn: &Connection) -> Result<Uuid> {
    let existing: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'device_id'", [], |r| {
            r.get(0)
        })
        .optional()?;
    match existing {
        Some(id) => Uuid::parse_str(&id).map_err(|e| StoreError::Invalid(e.to_string())),
        None => {
            let id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('device_id', ?1)",
                [id.to_string()],
            )?;
            Ok(id)
        }
    }
}

fn ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

fn from_ms(v: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(v).single().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn session(app: &str, url: Option<&str>, start: i64, end: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: "t".into(),
            url: url.map(Into::into),
            domain: url.and_then(crate::url_util::domain_of),
            started_at: t(start),
            ended_at: t(end),
            category_id: None,
            project_id: None,
        }
    }

    #[test]
    fn device_id_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("tracky-test-{}.db", Uuid::new_v4()));
        let first = Store::open(&dir).unwrap();
        let sync: i64 = first
            .conn()
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap();
        assert_eq!(sync, 1, "WAL ile synchronous=NORMAL");
        let first = first.device_id();
        let second = Store::open(&dir).unwrap().device_id();
        assert_eq!(first, second);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn upsert_updates_in_progress_session() {
        let store = Store::open_in_memory().unwrap();
        let mut s = session("Code", None, 0, 10);
        store.upsert_session(&s).unwrap();
        s.ended_at = t(60);
        store.upsert_session(&s).unwrap();
        let all = store.sessions_between(t(0), t(100)).unwrap();
        assert_eq!(all, vec![s]);
    }

    #[test]
    fn totals_are_clipped_to_range() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_session(&session("Code", None, 0, 100))
            .unwrap();
        store
            .upsert_session(&session("Code", None, 200, 300))
            .unwrap();
        store
            .upsert_session(&session("Chrome", Some("https://github.com/x"), 100, 200))
            .unwrap();
        store
            .upsert_session(&session("Chrome", Some("youtube.com/watch"), 300, 330))
            .unwrap();

        // [50, 250) aralığı: Code 50 + 50, Chrome 100.
        let apps = store.app_totals(t(50), t(250)).unwrap();
        let got: Vec<_> = apps.iter().map(|u| (u.label.as_str(), u.seconds)).collect();
        assert_eq!(got, [("Chrome", 100), ("Code", 100)]);

        let domains = store.domain_totals(t(0), t(1000)).unwrap();
        let got: Vec<_> = domains
            .iter()
            .map(|u| (u.key.as_str(), u.seconds))
            .collect();
        assert_eq!(got, [("github.com", 100), ("youtube.com", 30)]);
    }

    #[test]
    fn privacy_settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.privacy_settings().unwrap(),
            PrivacySettings::default()
        );
        let s = PrivacySettings {
            excluded_apps: vec!["com.1password.1password".into()],
            ..Default::default()
        };
        store.save_privacy_settings(&s).unwrap();
        store.save_privacy_settings(&s).unwrap();
        assert_eq!(store.privacy_settings().unwrap(), s);
    }

    #[test]
    fn title_totals_for_one_app() {
        let store = Store::open_in_memory().unwrap();
        let mut a = session("Safari", None, 0, 60);
        a.title = "GitHub".into();
        let mut b = session("Safari", None, 60, 90);
        b.title = "Gmail".into();
        let mut c = session("Safari", None, 90, 150);
        c.title = "GitHub".into();
        for s in [&a, &b, &c, &session("Code", None, 0, 500)] {
            store.upsert_session(s).unwrap();
        }
        let got: Vec<_> = store
            .title_totals("com.test.Safari", t(0), t(1000))
            .unwrap()
            .into_iter()
            .map(|u| (u.label, u.seconds))
            .collect();
        assert_eq!(
            got,
            [("GitHub".to_string(), 120), ("Gmail".to_string(), 30)]
        );
    }

    #[test]
    fn seeds_default_categories_once() {
        let store = Store::open_in_memory().unwrap();
        let tags = store.tags().unwrap();
        assert_eq!(tags.len(), DEFAULT_CATEGORIES.len());
        assert!(!store.rules().unwrap().is_empty());
        // Silinen varsayılan kategori yeniden eklenmez.
        store.delete_tag(&tags[0].id).unwrap();
        store.seed_default_tags().unwrap();
        assert_eq!(store.tags().unwrap().len(), DEFAULT_CATEGORIES.len() - 1);

        // Varsayılanlar en eski sürüm olarak tohumlanır.
        let max: i64 = store
            .conn
            .query_row(
                "SELECT MAX(updated_at) FROM tags WHERE deleted_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(max, 0);

        // İki kurulum aynı varsayılan kimlikleri üretir.
        let other = Store::open_in_memory().unwrap();
        let ids = |s: &Store| {
            s.tags()
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>()
        };
        assert!(ids(&store).iter().all(|id| ids(&other).contains(id)));
    }

    #[test]
    fn assign_app_category_replaces_existing_rule() {
        let store = Store::open_in_memory().unwrap();
        let tags = store.tags().unwrap();
        let comm = tags.iter().find(|t| t.name == "İletişim").unwrap();
        let dev = tags.iter().find(|t| t.name == "Geliştirme").unwrap();
        let mut s = session("x", None, 0, 60);
        s.app_id = "com.microsoft.teams2".into();
        store.upsert_session(&s).unwrap();

        let category = |store: &Store| {
            let r = store.report(t(0), t(100), &[t(0)], false).unwrap();
            r.apps[0].category_id.clone()
        };
        assert_eq!(category(&store).as_deref(), Some(comm.id.as_str()));
        store
            .assign_app_category("com.microsoft.teams2", Some(&dev.id))
            .unwrap();
        assert_eq!(category(&store).as_deref(), Some(dev.id.as_str()));
        store
            .assign_app_category("com.microsoft.teams2", None)
            .unwrap();
        assert_eq!(category(&store), None);
    }

    #[test]
    fn search_export_keeps_matching_sessions_clipped_to_range() {
        let store = Store::open_in_memory().unwrap();
        let mut hit = session("Code", None, 0, 7200);
        hit.title = "sync.rs — Tracky".into();
        let miss = session("Slack", None, 0, 600);
        store.upsert_session(&hit).unwrap();
        store.upsert_session(&miss).unwrap();
        let csv = store
            .export_search_csv("tracky", t(3600), t(10_000))
            .unwrap();
        let rows: Vec<&str> = csv.lines().skip(1).collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains(",3600,Code,"), "{}", rows[0]);
        assert_eq!(
            store
                .export_search_csv("  ", t(0), t(10_000))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn known_apps_are_listed_by_last_use_with_latest_name() {
        let store = Store::open_in_memory().unwrap();
        let mut a_old = session("a", None, 0, 60);
        a_old.app_name = "Eski Ad".into();
        let mut a_new = session("a", None, 500, 560);
        a_new.app_name = "Yeni Ad".into();
        let b = session("b", None, 200, 260);
        let deleted = session("c", None, 900, 960);
        for s in [&a_old, &a_new, &b, &deleted] {
            store.upsert_session(s).unwrap();
        }
        store.delete_session(&deleted.id).unwrap();
        let apps: Vec<_> = store
            .known_apps(10)
            .unwrap()
            .into_iter()
            .map(|u| (u.key, u.label))
            .collect();
        assert_eq!(
            apps,
            [
                ("com.test.a".to_string(), "Yeni Ad".to_string()),
                ("com.test.b".to_string(), "b".to_string())
            ]
        );
        assert_eq!(store.known_apps(1).unwrap().len(), 1);
    }

    #[test]
    fn timesheet_day_can_be_approved_edited_and_exported() {
        let store = Store::open_in_memory().unwrap();
        let mut work = session("Figma", None, 0, 3600);
        work.title = "Trumore Loyalty UI/UX — Figma".into();
        store.upsert_session(&work).unwrap();
        let p = store.accept_project_suggestion("Trumore").unwrap();
        store
            .save_timesheet_config(&TimesheetConfig {
                default_party: "ADBA".into(),
                ..Default::default()
            })
            .unwrap();
        let proposed = store.propose_timesheet(t(-36_000), t(36_000), &[]).unwrap();
        assert_eq!(proposed.len(), 1);
        assert_eq!(
            (proposed[0].division.as_str(), proposed[0].party.as_str()),
            ("Trumore", "ADBA")
        );
        assert_eq!(proposed[0].project_id, p.id);
        let day = proposed[0].date;

        // Onayla, düzenle, ekle.
        store.replace_timesheet_day(day, &proposed).unwrap();
        let saved = store.timesheet_entries(day, day).unwrap();
        let mut edited = saved[0].entry.clone();
        edited.details = "Loyalty ekranları".into();
        store
            .save_timesheet_entry(Some(&saved[0].id), &edited)
            .unwrap();
        let mut extra = edited.clone();
        extra.kind = EntryKind::F2F;
        extra.hours = 0.5;
        let extra_id = store.save_timesheet_entry(None, &extra).unwrap();
        assert_eq!(store.timesheet_entries(day, day).unwrap().len(), 2);
        assert!(
            store
                .save_timesheet_entry(
                    None,
                    &TimesheetEntry {
                        hours: 0.0,
                        ..extra.clone()
                    }
                )
                .is_err()
        );

        // Aktarılan kayıt korunur: değiştirilemez, gün yeniden önerilince silinmez.
        store
            .mark_timesheet_exported(&[saved[0].id.clone()], Utc::now())
            .unwrap();
        assert!(
            store
                .save_timesheet_entry(Some(&saved[0].id), &edited)
                .is_err()
        );
        store.replace_timesheet_day(day, &[]).unwrap();
        // Yeniden öneri aktarılmış işi ikinci kez önermez (dosyaya iki kez yazılırdı).
        store.replace_timesheet_day(day, &proposed).unwrap();
        let left = store.timesheet_entries(day, day).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].entry.details, "Loyalty ekranları");
        assert!(left[0].exported_at.is_some());
        assert!(left.iter().all(|e| e.id != extra_id));
    }

    #[test]
    fn projects_link_to_clients() {
        let store = Store::open_in_memory().unwrap();
        let p = store.accept_project_suggestion("Trumore").unwrap();
        let togg = Client {
            id: Uuid::new_v4().to_string(),
            name: " Togg ".into(),
        };
        store.upsert_client(&togg, 0).unwrap();
        assert_eq!(store.clients().unwrap()[0].name, "Togg");
        store.set_project_client(&p.id, Some(&togg.id)).unwrap();
        assert_eq!(store.project_clients().unwrap().get(&p.id), Some(&togg.id));
        // Projeyi yeniden adlandırmak müşterisini bozmaz.
        store
            .upsert_tag(
                &Tag {
                    name: "Trumore 2".into(),
                    ..p.clone()
                },
                0,
            )
            .unwrap();
        assert_eq!(store.project_clients().unwrap().get(&p.id), Some(&togg.id));
        // Olmayan müşteri ya da kategori bağlanamaz; boş ad kaydedilmez.
        assert!(store.set_project_client(&p.id, Some("yok")).is_err());
        let cat = store.tags().unwrap()[0].id.clone();
        assert!(store.set_project_client(&cat, Some(&togg.id)).is_err());
        assert!(
            store
                .upsert_client(
                    &Client {
                        id: "x".into(),
                        name: " ".into()
                    },
                    1
                )
                .is_err()
        );
        // Müşterisiz yapılabilir; müşteri silinince proje kalır, bağlantı kalkar.
        store.set_project_client(&p.id, None).unwrap();
        assert!(store.project_clients().unwrap().is_empty());
        store.set_project_client(&p.id, Some(&togg.id)).unwrap();
        store.delete_client(&togg.id).unwrap();
        assert!(store.clients().unwrap().is_empty());
        assert!(store.project_clients().unwrap().is_empty());
        assert!(store.tags().unwrap().iter().any(|t| t.id == p.id));
    }

    #[test]
    fn manual_project_overrides_rules_and_splits_at_range() {
        let store = Store::open_in_memory().unwrap();
        let mut code = session("Code", None, 0, 3600);
        code.title = "sync.rs — tracky".into();
        store.upsert_session(&code).unwrap();
        let kum = store.accept_project_suggestion("tracky").unwrap(); // kural: "tracky"
        let togg = Tag {
            id: Uuid::new_v4().to_string(),
            kind: TagKind::Project,
            name: "Trumore".into(),
            color: 2,
        };
        store.upsert_tag(&togg, 9).unwrap();
        let projects = |from: i64, to: i64| {
            let r = store.report(t(from), t(to), &[t(from)], false).unwrap();
            r.projects
                .into_iter()
                .map(|b| (b.id, b.seconds))
                .collect::<Vec<_>>()
        };
        // Son yarım saat elle Trumore'a: kural yalnızca ilk yarıda kalır.
        assert_eq!(
            store
                .set_project_between(t(1800), t(3600), Some(&togg.id))
                .unwrap(),
            1
        );
        let mut got = projects(0, 3600);
        got.sort();
        let mut want = vec![(Some(kum.id.clone()), 1800), (Some(togg.id.clone()), 1800)];
        want.sort();
        assert_eq!(got, want);
        // Kurallara döndürünce yine tamamı kurala göre.
        store.set_project_between(t(0), t(3600), None).unwrap();
        assert_eq!(projects(0, 3600), [(Some(kum.id.clone()), 3600)]);
        // "Projesiz": kurala uysa da ilk yarım saat hiçbir projeye sayılmaz; geri alınabilir.
        store
            .set_project_between(t(0), t(1800), Some(crate::classify::NO_PROJECT))
            .unwrap();
        let mut got = projects(0, 3600);
        got.sort();
        assert_eq!(got, [(None, 1800), (Some(kum.id.clone()), 1800)]);
        store.set_project_between(t(0), t(3600), None).unwrap();
        assert_eq!(projects(0, 3600), [(Some(kum.id.clone()), 3600)]);
        // Kategori kimliği proje olarak verilemez.
        let cat = store.tags().unwrap()[0].id.clone();
        assert!(store.set_project_between(t(0), t(10), Some(&cat)).is_err());
        // Elle kayıt projeyle eklenebilir.
        let m = store
            .add_manual_session("Toplantı", t(4000), t(5800), None, Some(&togg.id))
            .unwrap();
        assert_eq!(m.project_id.as_deref(), Some(togg.id.as_str()));
        assert_eq!(projects(4000, 5800), [(Some(togg.id.clone()), 1800)]);
    }

    #[test]
    fn long_sessions_are_found_by_range_queries() {
        // Aralık sorguları "en uzun oturum" alt sınırını kullanır; sonradan uzatılan
        // (takip) ya da uzaktan gelen uzun oturumlar da bulunmalı.
        let store = Store::open_in_memory().unwrap();
        let day = 86_400;
        store.upsert_session(&session("kisa", None, 0, 60)).unwrap();
        let mut long = session("uzun", None, 0, 60);
        store.upsert_session(&long).unwrap();
        long.ended_at = t(30 * day);
        store.upsert_session(&long).unwrap();
        let found = store.sessions_between(t(20 * day), t(21 * day)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].app_name, "uzun");
        assert_eq!(
            store.app_totals(t(20 * day), t(21 * day)).unwrap()[0].seconds,
            day
        );
        assert_eq!(store.delete_between(t(20 * day), t(21 * day)).unwrap(), 1);
        assert!(
            store
                .sessions_between(t(20 * day), t(21 * day))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn suggestions_can_be_accepted_or_dismissed() {
        let store = Store::open_in_memory().unwrap();
        let mut code = session("x", None, 0, 40 * 60);
        code.app_id = "com.microsoft.VSCode".into();
        code.title = "sync.rs — tracky".into();
        let mut postman = session("Postman", None, 40 * 60, 60 * 60);
        postman.app_id = "com.postmanlabs.mac".into();
        store.upsert_session(&code).unwrap();
        store.upsert_session(&postman).unwrap();

        let now = t(3600);
        let s = store.suggestions(now).unwrap();
        assert_eq!(s.projects[0].name, "tracky");
        assert_eq!(s.categories[0].label, "Postman");

        let tag = store.accept_project_suggestion("tracky").unwrap();
        let c = &s.categories[0];
        store
            .accept_category_suggestion(c.field, &c.pattern, &c.category_id)
            .unwrap();
        let after = store.suggestions(now).unwrap();
        assert!(after.projects.is_empty() && after.categories.is_empty());
        let report = store.report(t(0), now, &[t(0)], false).unwrap();
        assert_eq!(report.projects[0].id.as_deref(), Some(tag.id.as_str()));

        // Yoksayılan öneri bir daha gelmez.
        let other = Store::open_in_memory().unwrap();
        other.upsert_session(&code).unwrap();
        other.dismiss_suggestion(&s.projects[0].key).unwrap();
        other.dismiss_suggestion(&s.projects[0].key).unwrap();
        assert!(other.suggestions(now).unwrap().projects.is_empty());
    }

    #[test]
    fn updated_at_never_goes_backwards() {
        let store = Store::open_in_memory().unwrap();
        let tag = store.tags().unwrap()[0].clone();
        // Uzaktan, yerel saatten çok ileri bir sürüm gelmiş olsun.
        let future = ms(Utc::now()) + 3_600_000;
        store
            .conn
            .execute(
                "UPDATE tags SET updated_at = ?2 WHERE id = ?1",
                params![tag.id, future],
            )
            .unwrap();
        store
            .upsert_tag(
                &Tag {
                    name: "Yeni".into(),
                    ..tag.clone()
                },
                0,
            )
            .unwrap();
        let after: i64 = store
            .conn
            .query_row(
                "SELECT updated_at FROM tags WHERE id = ?1",
                [&tag.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(after > future);
    }

    #[test]
    fn generic_settings() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting::<bool>("onboarded").unwrap(), None);
        store.save_setting("onboarded", &true).unwrap();
        assert_eq!(store.setting::<bool>("onboarded").unwrap(), Some(true));
    }

    #[test]
    fn rejects_newer_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(Store::init(conn), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn manual_category_overrides_rules_and_can_be_cleared() {
        let store = Store::open_in_memory().unwrap();
        let tags = store.tags().unwrap();
        let design = tags
            .iter()
            .find(|t| t.name == "Tasarım")
            .unwrap()
            .id
            .clone();
        store
            .upsert_session(&session("VSCode", None, 0, 600))
            .unwrap();
        let mut vscode = session("Other", None, 700, 900);
        vscode.app_id = "com.microsoft.VSCode".into();
        store.upsert_session(&vscode).unwrap();

        let report = |s: &Store| s.report(t(0), t(3600), &[t(0)], false).unwrap();
        let dev = report(&store).categories;
        assert!(dev.iter().any(|b| b.id.is_some() && b.seconds == 200));

        // Yalnızca aralıkla kesişen oturum değişir.
        assert_eq!(
            store
                .set_category_between(t(650), t(1000), Some(&design))
                .unwrap(),
            1
        );
        let cats = report(&store).categories;
        assert!(cats.contains(&report::Bucket {
            id: Some(design.clone()),
            seconds: 200
        }));

        // Geri alınınca kurala döner; bilinmeyen etiket reddedilir.
        store.set_category_between(t(650), t(1000), None).unwrap();
        assert!(
            !report(&store)
                .categories
                .iter()
                .any(|b| b.id.as_deref() == Some(&*design))
        );
        assert!(
            store
                .set_category_between(t(0), t(10), Some("yok"))
                .is_err()
        );
    }

    #[test]
    fn delete_between_and_manual_sessions() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        store.upsert_session(&session("B", None, 600, 900)).unwrap();
        assert_eq!(store.delete_between(t(0), t(600)).unwrap(), 1);
        assert_eq!(store.app_totals(t(0), t(3600)).unwrap().len(), 1);

        // Çakışan elle kayıt reddedilir, boş aralığa eklenir.
        assert!(
            store
                .add_manual_session("Toplantı", t(800), t(1200), None, None)
                .is_err()
        );
        assert!(
            store
                .add_manual_session("  ", t(1000), t(1200), None, None)
                .is_err()
        );
        let cat = store.tags().unwrap()[0].id.clone();
        let s = store
            .add_manual_session("Toplantı", t(1000), t(1600), Some(&cat), None)
            .unwrap();
        let back = store.sessions_between(t(0), t(3600)).unwrap();
        let manual = back.iter().find(|x| x.id == s.id).unwrap();
        assert!(manual.is_manual());
        assert_eq!(manual.app_name, "Toplantı");
        assert_eq!(manual.category_id.as_deref(), Some(&*cat));
    }

    #[test]
    fn focus_timers_start_stop_and_complete() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.start_focus(0, t(0)).is_err());
        let a = store.start_focus(25, t(0)).unwrap();
        assert_eq!(store.active_focus().unwrap(), Some(a.clone()));
        assert!(store.complete_due_focus(t(24 * 60)).unwrap().is_none());

        // Yenisi başlayınca önceki o anda biter.
        let b = store.start_focus(50, t(10 * 60)).unwrap();
        let all = store.focus_timers_between(t(0), t(7200)).unwrap();
        assert_eq!(all[0].end, Some(t(10 * 60)));
        assert_eq!(all[1].id, b.id);

        // Süresi dolunca planlanan bitişte kapanır.
        let done = store.complete_due_focus(t(70 * 60)).unwrap().unwrap();
        assert_eq!(done.end, Some(t(60 * 60)));
        assert!(store.active_focus().unwrap().is_none());

        // Bir dakikadan kısa deneme kayıt bırakmaz.
        store.start_focus(25, t(80 * 60)).unwrap();
        assert!(store.stop_focus(t(80 * 60 + 30)).unwrap().is_none());
        assert_eq!(store.focus_timers_between(t(0), t(7200)).unwrap().len(), 2);
    }

    #[test]
    fn range_edits_split_sessions_at_the_boundaries() {
        let store = Store::open_in_memory().unwrap();
        let long = session("A", None, 0, 3000);
        store.upsert_session(&long).unwrap();
        // Ortadaki 1000–2000 silinir; baş ve son korunur, asıl kimlik sonda kalır.
        assert_eq!(store.delete_between(t(1000), t(2000)).unwrap(), 1);
        let left = store.sessions_between(t(0), t(4000)).unwrap();
        let spans: Vec<_> = left.iter().map(|s| (s.started_at, s.ended_at)).collect();
        assert_eq!(spans, [(t(0), t(1000)), (t(2000), t(3000))]);
        assert_eq!(left[1].id, long.id);

        // Kategori ataması da yalnızca aralığın içine uygulanır.
        let cat = store.tags().unwrap()[0].id.clone();
        store
            .set_category_between(t(2500), t(2600), Some(&cat))
            .unwrap();
        let all = store.sessions_between(t(0), t(4000)).unwrap();
        let tagged: Vec<_> = all
            .iter()
            .filter(|s| s.category_id.is_some())
            .map(|s| (s.started_at, s.ended_at))
            .collect();
        assert_eq!(tagged, [(t(2500), t(2600))]);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn running_session_resumes_after_its_block_is_deleted() {
        let store = Store::open_in_memory().unwrap();
        let mut running = session("A", None, 0, 600);
        store.upsert_session(&running).unwrap();
        store.delete_between(t(0), t(600)).unwrap();
        assert!(store.sessions_between(t(0), t(4000)).unwrap().is_empty());
        // Motor aynı oturumu uzatmaya devam eder: silinme anından itibaren geri gelir.
        let deleted_at: i64 = store
            .conn
            .query_row(
                "SELECT deleted_at FROM sessions WHERE id = ?1",
                [running.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        running.ended_at = from_ms(deleted_at) + chrono::Duration::seconds(30);
        store.upsert_session(&running).unwrap();
        let back = store
            .sessions_between(t(0), from_ms(deleted_at + 60_000))
            .unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].started_at, from_ms(deleted_at));
    }
}
