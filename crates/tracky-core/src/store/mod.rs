//! SQLite depolama. Bu dosyada bağlantı, işlemler ve ayarlar; alt modüllerde şema göçleri
//! (`schema`), oturumlar ve düzenlemeleri (`sessions`, `edits`), rapor sorguları (`reports`),
//! yedek ve bütünlük (`files`), sınıflandırma (`taxonomy`), zaman çizelgesi (`timesheet`) ve
//! bilgisayarlar (`devices`).

mod devices;
mod edits;
mod files;
mod reports;
mod schema;
mod sessions;
mod taxonomy;
mod timesheet;

use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::privacy::PrivacySettings;

pub use devices::{BlockDevice, DEVICE_KEY_PREFIX, DeviceTotal, KnownDevice};
pub use edits::EditSnapshot;
pub use files::BackupInfo;
pub use sessions::EditScope;
pub use taxonomy::TagExtras;
pub use timesheet::{
    DayRow, DayRows, SavedEntry, SplitMeetings, TimesheetContext, first_timesheet_id,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("veritabanı hatası: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("ayar okunamadı: {0}")]
    Json(#[from] serde_json::Error),
    #[error("geçersiz kayıt: {0}")]
    Invalid(String),
    /// Aralıkta başka bilgisayarda süren bir oturum var: o bilgisayar oturumu kaydetmeye
    /// devam ettiği için buradaki değişiklik ilk eşitlemede ezilirdi.
    #[error(
        "Bu aralıkta diğer bilgisayarda şu an süren bir oturum var; değişiklik eşitlemede \
         kaybolurdu. Oturum bittikten birkaç dakika sonra tekrar dene."
    )]
    ForeignLiveSession,
}

pub type Result<T> = std::result::Result<T, StoreError>;

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
    /// Bu açılışa özgü, diske yazılmayan kimlik (eşitlemede satırı yazanı tanımak için).
    /// Kopyalanan bir veritabanı (örn. Taşıma Yardımcısı) cihaz kimliğini de taşır;
    /// kalıcı kimlik kullanılsaydı iki kurulum birbirinin satırlarını hiç çekmezdi.
    instance_id: Uuid,
}

/// İç içe geçebilen işlem (SQLite `SAVEPOINT`). Dışarıda açık bir işlem yoksa kendisi başlatır
/// ve `commit` ile yazar; varsa onun içinde bir kayıt noktası olur, yazılanlar dış işlemle
/// birlikte kalır ya da geri alınır. `commit` edilmeden düşerse yaptıkları geri alınır.
/// (`Connection::unchecked_transaction` iç içe açılamaz; depo işlemleri bu yüzden bunu kullanır.)
pub(crate) struct Savepoint<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> Savepoint<'a> {
    fn new(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch("SAVEPOINT kum")?;
        Ok(Self { conn, done: false })
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        self.conn.execute_batch("RELEASE kum")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if !self.done {
            // Hata zaten bildiriliyor; geri alma da başarısız olursa yapılacak bir şey yok.
            let _ = self.conn.execute_batch("ROLLBACK TO kum; RELEASE kum");
        }
    }
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

    /// İç içe geçebilen bir işlem açar (bkz. [`Savepoint`]).
    pub(crate) fn savepoint(&self) -> Result<Savepoint<'_>> {
        Savepoint::new(&self.conn)
    }

    /// `f`'yi tek işlemde çalıştırır: hata dönerse yaptığı her değişiklik geri alınır. Depo
    /// işlemleri iç içe geçebildiği için birkaç düzenlemeyi (örn. kural ekleyip oturumları
    /// atamak) hep ya da hiç olarak birleştirir.
    pub fn atomic<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let sp = self.savepoint()?;
        let out = f(self)?;
        sp.commit()?;
        Ok(out)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&mut conn)?;
        let device_id = device_id(&conn)?;
        let store = Self {
            conn,
            device_id,
            instance_id: Uuid::new_v4(),
        };
        store.seed_default_tags()?;
        store.migrate_timesheets()?;
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
             ON CONFLICT (key) DO UPDATE SET value = excluded.value,
               updated_at = MAX(excluded.updated_at, settings.updated_at + 1)
             WHERE settings.value IS NOT excluded.value",
            params![key, serde_json::to_string(value)?, ms(Utc::now())],
        )?;
        Ok(())
    }

    /// Eşitleme imleçlerini siler: sonraki eşitleme sunucudaki satırları baştan çeker
    /// (yeniden uygulamak zararsız, yerelde daha yeni olanlar korunur).
    pub fn forget_sync_cursors(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key LIKE 'sync_cursor:%'", [])?;
        Ok(())
    }

    /// Senkronizasyon başka hesaba/projeye bağlandığında: imleçleri sil, her şeyi
    /// yeniden gönderilecek işaretle. Eşitlenen ayarlar en eski sayılır: hesapta kayıtlı
    /// ayarlar (örn. diğer Mac'te kurulan zaman çizelgeleri) bu cihazın varsayılanlarını
    /// ezer; hesapta olmayanlar gönderilir.
    pub fn reset_sync_state(&self) -> Result<()> {
        let keys: Vec<String> = crate::sync::SYNCED_SETTINGS
            .iter()
            .map(|k| format!("'{k}'"))
            .collect();
        let tx = self.savepoint()?;
        self.conn.execute_batch(&format!(
            "DELETE FROM settings WHERE key LIKE 'sync_cursor:%';
             UPDATE sessions SET synced_at = NULL;
             UPDATE tags SET synced_at = NULL;
             UPDATE rules SET synced_at = NULL;
             UPDATE clients SET synced_at = NULL;
             UPDATE timesheet_entries SET synced_at = NULL;
             UPDATE settings SET synced_at = NULL, updated_at = 0
             WHERE key IN ({}) OR key LIKE '{}%';",
            keys.join(", "),
            devices::DEVICE_KEY_PREFIX
        ))?;
        tx.commit()
    }
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
mod tests;
