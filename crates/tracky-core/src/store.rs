use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::model::Session;
use crate::privacy::PrivacySettings;

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
];

const PRIVACY_KEY: &str = "privacy";

/// Bir zaman aralığında bir anahtar (uygulama, domain...) için toplam süre.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UsageTotal {
    pub key: String,
    pub label: String,
    pub seconds: i64,
}

pub struct Store {
    conn: Connection,
    device_id: Uuid,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        let device_id = device_id(&conn)?;
        Ok(Self { conn, device_id })
    }

    /// Bu kurulumun kalıcı kimliği; senkronizasyonda kayıtların kaynağını belirtir.
    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    /// Kayıtlı gizlilik ayarları; hiç kaydedilmediyse varsayılanlar.
    pub fn privacy_settings(&self) -> Result<PrivacySettings> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [PRIVACY_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default())
    }

    pub fn save_privacy_settings(&self, settings: &PrivacySettings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![PRIVACY_KEY, serde_json::to_string(settings)?, ms(Utc::now())],
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
                ended_at = excluded.ended_at,
                updated_at = excluded.updated_at",
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
        let mut stmt = self.conn.prepare(
            "SELECT id, app_id, app_name, title, url, domain, started_at, ended_at
             FROM sessions
             WHERE deleted_at IS NULL AND started_at < ?2 AND ended_at > ?1
             ORDER BY started_at",
        )?;
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
                    SUM(MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000 AS secs
             FROM sessions
             WHERE deleted_at IS NULL AND {key_col} IS NOT NULL
               AND started_at < ?2 AND ended_at > ?1
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
        }
    }

    #[test]
    fn device_id_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("tracky-test-{}.db", Uuid::new_v4()));
        let first = Store::open(&dir).unwrap().device_id();
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
    fn rejects_newer_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(Store::init(conn), Err(StoreError::Invalid(_))));
    }
}
