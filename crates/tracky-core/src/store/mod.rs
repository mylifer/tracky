//! SQLite depolama: göçler, oturumlar, ayarlar ve raporlama sorguları. Sınıflandırma
//! (etiketler, kurallar, müşteriler, öneriler) `taxonomy`, zaman çizelgesi `timesheet`
//! alt modülündedir.

mod devices;
mod edits;
mod taxonomy;
mod timesheet;

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::classify::{Classifier, TagKind};
use crate::model::{IDLE_APP_ID, MANUAL_APP_ID, Session};
use crate::privacy::PrivacySettings;
use crate::report::{self, Report};

pub use devices::{BlockDevice, DEVICE_KEY_PREFIX, DeviceTotal, KnownDevice};
pub use edits::EditSnapshot;
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
    r#"
-- Odak zamanlayıcısı kaldırıldı.
DROP TABLE focus_timers;
"#,
    r#"
-- Kurallar tarayıcı adresine de bakabilir (field = 'domain'). SQLite CHECK kısıtını
-- değiştiremediği için tablo yeniden kurulur; satırlar ve eşitleme durumu korunur.
CREATE TABLE rules_new (
    id         TEXT PRIMARY KEY,
    tag_id     TEXT NOT NULL REFERENCES tags (id),
    field      TEXT NOT NULL CHECK (field IN ('app', 'title', 'domain')),
    pattern    TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    synced_at  INTEGER
);
INSERT INTO rules_new (rowid, id, tag_id, field, pattern, position, updated_at, deleted_at, synced_at)
    SELECT rowid, id, tag_id, field, pattern, position, updated_at, deleted_at, synced_at FROM rules;
DROP TABLE rules;
ALTER TABLE rules_new RENAME TO rules;
CREATE INDEX rules_tag ON rules (tag_id);
"#,
    r#"
-- Proje arşivi (arşivlenme anı, ms) ve sözleşme bütçesi (adam-gün); müşterinin de bütçesi
-- olabilir. Üçü de eşitlenir (supabase/migrations/0007).
ALTER TABLE tags ADD COLUMN archived_at INTEGER;
ALTER TABLE tags ADD COLUMN budget_days REAL;
ALTER TABLE clients ADD COLUMN budget_days REAL;
"#,
    r#"
-- Zaman çizelgesi satırının kapsadığı takip aralıkları (JSON [[başlangıç, bitiş], …], unix ms;
-- boş dizi: elle eklenen satır, NULL: aralığı bilinmeyen eski satır), aktarıldığı çizelge ve
-- gizlenme (silinme) anı. Kaydedilmiş satırların aralıkları yeniden önerilmez.
ALTER TABLE timesheet_entries ADD COLUMN coverage TEXT;
ALTER TABLE timesheet_entries ADD COLUMN timesheet_id TEXT;
ALTER TABLE timesheet_entries ADD COLUMN dismissed_at INTEGER;
"#,
    r#"
-- Cihazdan bağımsız ayarlar da eşitlenir (crate::sync::SYNCED_SETTINGS; supabase/migrations/0008).
-- Ayar silinmez (null yazılır); deleted_at sunucu şemasıyla aynı olsun diye var.
ALTER TABLE settings ADD COLUMN deleted_at INTEGER;
ALTER TABLE settings ADD COLUMN synced_at INTEGER;
"#,
    r#"
-- Zaman çizelgesi satırları da eşitlenir (supabase/migrations/0010): ikinci bilgisayarda da
-- kaydedilen, gönderilen ve silinen satırlar görünür, aynı iş iki kez gönderilmez. Silme
-- yumuşaktır (deleted_at); eski satırlar ilk eşitlemede gönderilir.
ALTER TABLE timesheet_entries ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE timesheet_entries ADD COLUMN deleted_at INTEGER;
ALTER TABLE timesheet_entries ADD COLUMN synced_at INTEGER;
UPDATE timesheet_entries SET updated_at = created_at;
CREATE INDEX timesheet_entries_unsynced ON timesheet_entries (updated_at)
    WHERE synced_at IS NULL OR synced_at < updated_at;
"#,
    r#"
-- Satırın durumu (aktarıldı, gizlendi, silindi) içerikten ayrı zamanla eşitlenir (state_at;
-- supabase/migrations/0011): eşitlenmemiş başka cihazdaki eski kopyanın düzenlenmesi aktarımı
-- ya da silmeyi geri almasın. consultant: aktarılan satırın dosyaya yazıldığı danışman adı;
-- ayarlarda ad değişse de satır dosyada bulunur (şimdiki adla doldurulur).
ALTER TABLE timesheet_entries ADD COLUMN state_at INTEGER;
ALTER TABLE timesheet_entries ADD COLUMN consultant TEXT;
UPDATE timesheet_entries SET state_at = updated_at;
UPDATE timesheet_entries SET consultant = (
    SELECT NULLIF(TRIM(json_extract(t.value, '$.consultant')), '')
    FROM settings s, json_each(s.value, '$.timesheets') t
    WHERE s.key = 'timesheet' AND json_valid(s.value)
      AND json_extract(t.value, '$.id') = timesheet_entries.timesheet_id)
WHERE exported_at IS NOT NULL;
"#,
    r#"
-- Oturumun ataması (kategori, proje) ve silinmesi içerikten ayrı zamanla eşitlenir (state_at;
-- supabase/migrations/0012): başka bilgisayarda süren oturumun takipçe uzatılması, bu arada
-- yapılan atamayı ya da silmeyi geri almasın. Boş: hiç düzenlenmedi (en eski sayılır).
ALTER TABLE sessions ADD COLUMN state_at INTEGER;
"#,
    r#"
-- Takvim bloğunun elle bölündüğü an (Session::block_from; supabase/migrations/0013): kısaltılan
-- bloğun kesilen kısmı silinmez, bu anda başlayan oturumdan ayrı blok olur. Atama gibi state_at
-- ile eşitlenir.
ALTER TABLE sessions ADD COLUMN block_from INTEGER;
"#,
];

/// Yedek dosyasının içeriği (geri yüklemeden önce göstermek için).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub sessions: i64,
    pub last_activity: Option<DateTime<Utc>>,
}

/// `[?1, ?2)` ile kesişen oturumlar. Üçüncü koşul sonucu değiştirmez (kesişen her oturum
/// en uzun oturumdan kısadır), yalnızca indeksin alt sınırıdır.
const OVERLAPS: &str = "started_at < ?2 AND ended_at > ?1
    AND started_at >= ?1 - (SELECT max_duration FROM session_stats)";

/// Aralık düzenlemesinin yalnızca bazı uygulamalara (ve isteğe bağlı başlıklarına) uygulanması:
/// uygulama çizelgesinde bir uygulamanın çubuğuna tıklanınca aynı dilimdeki öteki uygulamalar
/// değişmesin.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditScope {
    pub app_ids: Vec<String>,
    /// Doluysa yalnızca bu pencere başlıkları.
    pub titles: Option<Vec<String>>,
}

/// `scope`'un SQL parametreleri (JSON dizileri; kapsam yoksa NULL): [`in_scope`] için.
fn scope_params(scope: Option<&EditScope>) -> Result<(Option<String>, Option<String>)> {
    let Some(scope) = scope else {
        return Ok((None, None));
    };
    if scope.app_ids.is_empty() {
        return Err(StoreError::Invalid("uygulama seçilmedi".into()));
    }
    Ok((
        Some(serde_json::to_string(&scope.app_ids)?),
        scope
            .titles
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
    ))
}

/// `?n` (uygulamalar) ve `?n+1` (başlıklar) parametreleriyle kapsam koşulu.
fn in_scope(n: usize) -> String {
    format!(
        "(?{n} IS NULL OR app_id IN (SELECT value FROM json_each(?{n})))
         AND (?{m} IS NULL OR title IN (SELECT value FROM json_each(?{m})))",
        m = n + 1
    )
}

/// Başka cihazın oturumu, bitişi bu kadar yakın olduğu sürece "sürüyor" sayılır. Cihazlar
/// 5 dakikada bir eşitlediği için buradaki kopya o cihazın gerçek durumundan ~10 dakika
/// geride olabilir.
const FOREIGN_LIVE_WINDOW_MS: i64 = 15 * 60 * 1000;

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

    /// Veritabanının tutarlı, sıkıştırılmış bir kopyasını `path`'e yazar (takip yazarken de
    /// güvenli). Dosya zaten varsa hata verir.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        self.conn
            .execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        Ok(())
    }

    /// `path` bu sürümün açabileceği bir Kum veritabanı mı? Dosyayı değiştirmeden okur;
    /// içindeki oturum sayısını ve son kaydın zamanını döndürür.
    pub fn inspect_backup(path: &Path) -> Result<BackupInfo> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let not_kum = || StoreError::Invalid("bu dosya bir Kum yedeği değil".into());
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| not_kum())?;
        if version < 1 {
            return Err(not_kum());
        }
        if usize::try_from(version).unwrap_or(usize::MAX) > MIGRATIONS.len() {
            return Err(StoreError::Invalid(
                "yedek Kum'un daha yeni bir sürümüyle alınmış; önce Kum'u güncelle".into(),
            ));
        }
        // Başlığı sağlam ama sayfaları bozuk (yarım kopyalanmış) bir dosya yerine konmasın.
        let check: String = conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
            .map_err(|_| not_kum())?;
        if check != "ok" {
            return Err(StoreError::Invalid("yedek dosyası bozuk".into()));
        }
        let (sessions, last): (i64, Option<i64>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(ended_at) FROM sessions WHERE deleted_at IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| not_kum())?;
        Ok(BackupInfo {
            sessions,
            last_activity: last.map(from_ms),
        })
    }

    /// `source` veritabanının tutarlı bir kopyasını `target`'a yazar (geri yükleme için).
    /// Dosya kopyalamanın aksine yanındaki `-wal` dosyasındaki son değişiklikler de kopyaya
    /// girer. `source` değiştirilmez; `target` zaten varsa hata verir.
    pub fn copy_database(source: &Path, target: &Path) -> Result<()> {
        let conn = Connection::open_with_flags(
            source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;
        Ok(())
    }

    /// Kapalı bir veritabanının `-wal` dosyasını ana dosyaya işler ve boşaltır; ana dosya
    /// tek başına eksiksiz olur (örn. kenara taşınmadan önce).
    pub fn checkpoint_file(path: &Path) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }

    /// Yedekten geri yüklenen veritabanı için: imleçleri siler ve eşitlenen tüm satırları
    /// şimdiki zamanla değişmiş sayar. Yedekteki eski "gönderildi" işaretleri ve imleçlerle
    /// eşitleme, sunucudaki (geri yüklemeden sonraki) sürümleri çekip geri yüklemeyi ezerdi;
    /// böylece geri yüklenen sürümler gönderilir ve "son yazan kazanır"da onlar kazanır.
    pub fn mark_restored_for_sync(&self) -> Result<()> {
        let tx = self.savepoint()?;
        self.conn
            .execute("DELETE FROM settings WHERE key LIKE 'sync_cursor:%'", [])?;
        let now = ms(Utc::now());
        for table in [
            "sessions",
            "tags",
            "rules",
            "clients",
            "settings",
            "timesheet_entries",
        ] {
            self.conn.execute(
                &format!(
                    "UPDATE {table} SET updated_at = MAX(updated_at + 1, ?1), synced_at = NULL"
                ),
                [now],
            )?;
        }
        tx.commit()?;
        Ok(())
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
                state_at = CASE WHEN sessions.deleted_at IS NULL THEN sessions.state_at
                    ELSE excluded.updated_at END,
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

    /// Toplamlar için: `[from, to)` ile kesişen çalışma oturumları, bilgisayarlar arası
    /// çakışmalar bir kez sayılacak şekilde birleştirilmiş ([`crate::model::merge_devices`]).
    /// Atanmamış boşta kayıtları dahil değildir ([`Session::counts_as_work`]).
    pub fn merged_sessions_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Session>> {
        let mut sessions = self.merged_sessions_with_idle_between(from, to)?;
        sessions.retain(Session::counts_as_work);
        Ok(sessions)
    }

    /// Takvim için: [`Self::merged_sessions_between`] gibi, ama boşta kayıtlarıyla.
    pub fn merged_sessions_with_idle_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Session>> {
        Ok(crate::model::merge_devices(
            self.sessions_between(from, to)?,
        ))
    }

    /// `[from, to)` ile kesişen oturumlar (ham, cihazlar üst üste binebilir), başlangıca göre sıralı.
    pub fn sessions_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Session>> {
        Ok(self
            .sessions_with_devices_between(from, to)?
            .into_iter()
            .map(|(s, _)| s)
            .collect())
    }

    /// [`Self::sessions_between`], her oturumun bilgisayarıyla.
    fn sessions_with_devices_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(Session, String)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, app_id, app_name, title, url, domain, started_at, ended_at, category_id,
                    project_id, device_id, block_from
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
                    block_from: r.get::<_, Option<i64>>(11)?.map(from_ms),
                },
                r.get::<_, String>(10)?,
            ))
        })?;
        rows.map(|row| {
            let (id, mut s, device) = row?;
            s.id = Uuid::parse_str(&id).map_err(|e| StoreError::Invalid(e.to_string()))?;
            Ok((s, device))
        })
        .collect()
    }

    /// Uygulama başına toplam süre; aralık dışına taşan kısımlar kırpılır.
    pub fn app_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| Some((&s.app_id, &s.app_name)))
    }

    /// Domain başına toplam süre (yalnızca URL'si bilinen oturumlar).
    pub fn domain_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| s.domain.as_ref().map(|d| (d, d)))
    }

    /// Oturumu yumuşak siler (örn. boşta kalma sonrası geçersiz kalan kayıt).
    pub fn delete_session(&self, id: &Uuid) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "UPDATE sessions SET deleted_at = ?2, state_at = ?2, updated_at = MAX(?2, updated_at + 1)
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
        self.set_category_in(from, to, category_id, None)
    }

    /// [`Self::set_category_between`]; `scope` verilirse yalnızca o uygulamaların (ve
    /// başlıkların) oturumları.
    pub fn set_category_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        if let Some(id) = category_id {
            self.require_tag(id, TagKind::Category)?;
        }
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET category_id = ?3, state_at = ?4, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND category_id IS NOT ?3 AND {scope}",
                scope = in_scope(5)
            ),
            params![ms(from), ms(to), category_id, ms(Utc::now()), apps, titles],
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
        self.set_project_in(from, to, project_id, None)
    }

    /// [`Self::set_project_between`]; `scope` verilirse yalnızca o uygulamaların (ve
    /// başlıkların) oturumları.
    pub fn set_project_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        project_id: Option<&str>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        if let Some(id) = project_id.filter(|id| *id != crate::classify::NO_PROJECT) {
            self.require_tag(id, TagKind::Project)?;
        }
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET project_id = ?3, state_at = ?4, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND project_id IS NOT ?3 AND {scope}",
                scope = in_scope(5)
            ),
            params![ms(from), ms(to), project_id, ms(Utc::now()), apps, titles],
        )?;
        // Raporda bilerek atanan süre, projenin silinmiş satırında kalsa da yeniden önerilir.
        if let Some(id) = project_id.filter(|id| *id != crate::classify::NO_PROJECT) {
            self.forget_dismissed(id, from, to)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` içindeki süreyi yumuşak siler; sınırı aşan oturumların dışarıda
    /// kalan kısmı korunur. Silinen satır sayısı.
    pub fn delete_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        self.delete_in(from, to, None)
    }

    /// [`Self::delete_between`]; `scope` verilirse yalnızca o uygulamaların (ve başlıkların)
    /// oturumları.
    pub fn delete_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET deleted_at = ?3, state_at = ?3, updated_at = MAX(?3, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS} AND {scope}",
                scope = in_scope(4)
            ),
            params![ms(from), ms(to), ms(Utc::now()), apps, titles],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// Takvim bloğunu `[from, to)` aralığından `[new_from, new_to)` aralığına uzatır ya da
    /// kısaltır. Kısaltınca blok ikiye bölünür: dışarıda kalan kısım silinmez, kategorisi ve
    /// projesiyle ayrı blok (ayrı çizelge satırı) olur ([`Session::block_from`]). Bloğa
    /// katılan kısımdaki kayıtlar bloğun kategorisini ve projesini alır, aradaki bölmeler
    /// kalkar; kaydı olmayan boşluklar (bilgisayar başında olunmayan süre) aynı kategori ve
    /// projede `label` adlı elle kayıtla dolar: zaman çizelgesine bloğun yeni aralığı gider.
    /// Projesi olmayan blok projeli işin üstüne uzarsa o işin projesini alır: iki blok tek
    /// blok (tek çizelge satırı) olur. Değişen satır sayısı.
    #[allow(clippy::too_many_arguments)]
    pub fn resize_block(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        new_from: DateTime<Utc>,
        new_to: DateTime<Utc>,
        label: &str,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        if new_to <= new_from {
            return Err(StoreError::Invalid(
                "bitiş başlangıçtan sonra olmalı".into(),
            ));
        }
        if new_to > Utc::now() {
            return Err(StoreError::Invalid(
                "blok henüz gelmemiş bir zamana uzatılamaz".into(),
            ));
        }
        let joined = [(new_from, from.min(new_to)), (to.max(new_from), new_to)];
        let adopted = match project_id {
            Some(_) => None,
            None => self.joined_project(&joined)?,
        };
        let project_id = project_id.or(adopted.as_deref());
        let tx = self.savepoint()?;
        let mut n = 0;
        if new_from < from || new_to > to {
            n += self.clear_block_starts(new_from, new_to)?;
        }
        // Kesilen baştan sonra kalan blok, kesilen sondan sonra kesilen parça yeni blok başlatır.
        if new_from > from && new_from < to {
            n += self.mark_block_start(new_from, to.min(new_to))?;
        }
        if new_to < to && new_to > from {
            n += self.mark_block_start(new_to.max(new_from), to)?;
        }
        for (a, b) in joined {
            if b > a {
                n += self.join_block(a, b, label, category_id, project_id)?;
            }
        }
        // Bloğun kendi kısmı da katıldığı işin projesine geçer.
        let (a, b) = (from.max(new_from), to.min(new_to));
        if adopted.is_some() && b > a {
            n += self.set_project_between(a, b, project_id)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` içinde başlayan ilk çalışma oturumundan yeni takvim bloğu başlatır
    /// ([`Session::block_from`]); `from`'u aşan oturum önce bölünür. Değişen satır sayısı.
    fn mark_block_start(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        self.ensure_no_foreign_live(from, to)?;
        self.split_at(from, to)?;
        let Some(first) = self
            .sessions_between(from, to)?
            .into_iter()
            .find(|s| s.started_at >= from && s.counts_as_work())
        else {
            return Ok(0);
        };
        let now = ms(Utc::now());
        Ok(self.conn.execute(
            "UPDATE sessions SET block_from = started_at, state_at = ?2,
                 updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1",
            params![first.id.to_string(), now],
        )?)
    }

    /// `(from, to)` içindeki elle bölmeleri kaldırır (aralığın başındaki kalır): aralık tek
    /// blok olur. Değişen satır sayısı.
    fn clear_block_starts(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        let now = ms(Utc::now());
        Ok(self.conn.execute(
            "UPDATE sessions SET block_from = NULL, state_at = ?3,
                 updated_at = MAX(?3, updated_at + 1)
             WHERE deleted_at IS NULL AND block_from > ?1 AND block_from < ?2",
            params![ms(from), ms(to), now],
        )?)
    }

    /// [`Self::resize_block`]'ta projesi olmayan bloğa katılan aralıklarda en çok süren proje
    /// ("Projesiz" sayılmaz).
    fn joined_project(&self, ranges: &[(DateTime<Utc>, DateTime<Utc>)]) -> Result<Option<String>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut by_project: HashMap<String, i64> = HashMap::new();
        for &(from, to) in ranges.iter().filter(|(a, b)| b > a) {
            for s in self.sessions_between(from, to)? {
                if !s.counts_as_work() {
                    continue;
                }
                let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
                if let (Some(p), true) = (classifier.classify(&s).project, ms > 0) {
                    *by_project.entry(p).or_default() += ms;
                }
            }
        }
        Ok(by_project
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(p, _)| p))
    }

    /// [`Self::resize_block`]'ta bloğa katılan `[from, to)` aralığı.
    fn join_block(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        label: &str,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        let mut n = 0;
        if category_id.is_some() {
            n += self.set_category_between(from, to, category_id)?;
        }
        if project_id.is_some() {
            n += self.set_project_between(from, to, project_id)?;
        }
        // Atamadan sonra hâlâ çalışma sayılmayan (kaydı olmayan ya da boşta) kısımlar.
        let mut busy: Vec<_> = self
            .sessions_between(from, to)?
            .into_iter()
            .filter(Session::counts_as_work)
            .map(|s| (s.started_at.max(from), s.ended_at.min(to)))
            .collect();
        busy.sort();
        busy.push((to, to));
        let manual_project = project_id.filter(|p| *p != crate::classify::NO_PROJECT);
        let mut at = from;
        for (a, b) in busy {
            if a - at >= Duration::minutes(1) {
                self.add_manual_session(label, at, a, category_id, manual_project)?;
                n += 1;
            }
            at = at.max(b);
        }
        Ok(n)
    }

    /// Aralıkta başka cihazın hâlâ sürüyor olabilecek oturumu varsa düzenlemeyi reddeder.
    /// O cihaz süren oturumu birkaç saniyede bir yeniden kaydeder; satırın tamamı son
    /// yazanla eşitlendiği için buradaki bölme, silme ya da atama geri alınırdı.
    /// Elle eklenen kayıtlar ve boşta kayıtları sürmez (bitince bir kez yazılır), kapsam dışıdır.
    fn ensure_no_foreign_live(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        let live: bool = self.conn.query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM sessions
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND device_id != ?3 AND ended_at > ?4
                   AND substr(app_id, 1, length(?5)) != ?5 AND app_id != ?6)"
            ),
            params![
                ms(from),
                ms(to),
                self.device_id.to_string(),
                ms(Utc::now()) - FOREIGN_LIVE_WINDOW_MS,
                format!("{MANUAL_APP_ID}/"),
                IDLE_APP_ID,
            ],
            |r| r.get(0),
        )?;
        if live {
            return Err(StoreError::ForeignLiveSession);
        }
        Ok(())
    }

    /// `[from, to)` sınırını aşan oturumları sınırlarda böler; sonra aralıkla
    /// kesişen her oturum tamamen aralığın içindedir. Asıl kimlik en son parçada
    /// kalır: süren oturumu takip eden motor doğru satırı uzatmaya devam eder.
    fn split_at(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        // Boş aralık sıfır uzunlukta parça, ters aralık ham bir CHECK hatası üretirdi.
        if to <= from {
            return Err(StoreError::Invalid(
                "aralığın sonu başlangıcından sonra olmalı".into(),
            ));
        }
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
                         category_id, project_id, state_at, block_from, started_at, ended_at, updated_at)
                     SELECT ?2, device_id, app_id, app_name, title, url, domain, category_id,
                         project_id, state_at, block_from, ?3, ?4, ?5
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
        // Takip edilen süreyle çakışırsa aynı dakikalar iki kez sayılırdı. Atanmamış boşta
        // kaydının yerini ise elle kayıt alır (boşta geçen süreyi açıklamanın yolu bu).
        let overlapping = self.sessions_between(from, to)?;
        if overlapping.iter().any(Session::counts_as_work) {
            return Err(StoreError::Invalid(
                "bu aralıkta zaten kayıt var; önce o bloğu silin".into(),
            ));
        }
        let tx = self.savepoint()?;
        if !overlapping.is_empty() {
            self.split_at(from, to)?;
            self.conn.execute(
                &format!(
                    "UPDATE sessions SET deleted_at = ?3, state_at = ?3, updated_at = MAX(?3, updated_at + 1)
                     WHERE deleted_at IS NULL AND {OVERLAPS} AND app_id = ?4"
                ),
                params![ms(from), ms(to), ms(Utc::now()), IDLE_APP_ID],
            )?;
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
            block_from: None,
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
        tx.commit()?;
        Ok(session)
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

    /// `[from, to)` aralığında başlığında ya da uygulama adında `query` geçen süre.
    pub fn search(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
    ) -> Result<crate::search::SearchResult> {
        let sessions = self.merged_sessions_between(from, to)?;
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
            .merged_sessions_between(from, to)?
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

    /// `[from, to)` aralığında projenin profili; `day_starts` yerel gün sınırları,
    /// `utc_offset` bir anın yerel saat farkı (saniye).
    pub fn project_stats(
        &self,
        project: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        utc_offset: impl Fn(DateTime<Utc>) -> i32,
    ) -> Result<crate::project_stats::ProjectStats> {
        let sessions = self.merged_sessions_between(from, to)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        Ok(crate::project_stats::build(
            &sessions,
            &classifier,
            project,
            from,
            to,
            day_starts,
            utc_offset,
        ))
    }

    /// Ardışık dönemlerde proje ve kategori süreleri; `bounds` dönem sınırları (n + 1 öğe).
    pub fn trends(&self, bounds: &[DateTime<Utc>]) -> Result<crate::trends::Trends> {
        let (Some(first), Some(last)) = (bounds.first(), bounds.last()) else {
            return Ok(Default::default());
        };
        let sessions = self.merged_sessions_between(*first, *last)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        Ok(crate::trends::trends(&sessions, &classifier, bounds))
    }

    /// Tüm çalışma oturumlarının CSV dökümü (atanmamış boşta kayıtları hariç).
    pub fn export_csv(&self) -> Result<String> {
        let mut sessions =
            self.sessions_between(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC)?;
        sessions.retain(Session::counts_as_work);
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
        self.report_for_device(from, to, day_starts, with_timeline, None)
    }

    /// Birleştirilmiş oturumlardan rapor.
    fn build_report(
        &self,
        sessions: &[Session],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        with_timeline: bool,
    ) -> Result<Report> {
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(report::build(
            sessions,
            &tags,
            &classifier,
            from,
            to,
            day_starts,
            with_timeline,
        ))
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
        // Milisaniye toplanır, en sonda saniyeye çevrilir (oturum başına kırpılmaz).
        let mut out: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for s in self.merged_sessions_between(from, to)? {
            let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
            if let (Some(id), true) = (pick(classifier.classify(&s)), ms > 0) {
                *out.entry(id).or_default() += ms;
            }
        }
        out.values_mut().for_each(|v| *v /= 1000);
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
             WHERE s.app_id != ?2
             ORDER BY s.ended_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64, IDLE_APP_ID], |r| {
            Ok(UsageTotal {
                key: r.get(0)?,
                label: r.get(1)?,
                seconds: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Bir uygulamanın pencere başlıklarına göre süre dağılımı (cihazlar arası çakışmalar
    /// [`Self::app_totals`] gibi bir kez sayılır).
    pub fn title_totals(
        &self,
        app_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| {
            (s.app_id == app_id).then_some((&s.title, &s.title))
        })
    }

    /// `key` ile gruplanmış toplam süre. Bilgisayarlar arası çakışmalar bir kez sayılır ve
    /// süreler milisaniye olarak toplanıp en sonda saniyeye çevrilir (oturum başına kırpılmaz).
    fn totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        key: impl Fn(&Session) -> Option<(&String, &String)>,
    ) -> Result<Vec<UsageTotal>> {
        let sessions = self.merged_sessions_between(from, to)?;
        // anahtar → (en son görülen ad, milisaniye)
        let mut sums: HashMap<&String, (&String, i64)> = HashMap::new();
        for s in &sessions {
            let Some((k, label)) = key(s) else { continue };
            let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
            if ms <= 0 {
                continue;
            }
            let entry = sums.entry(k).or_insert((label, 0));
            entry.0 = label;
            entry.1 += ms;
        }
        let mut out: Vec<UsageTotal> = sums
            .into_iter()
            .map(|(k, (label, ms))| UsageTotal {
                key: k.clone(),
                label: label.clone(),
                seconds: ms / 1000,
            })
            .collect();
        out.sort_by(|a, b| {
            b.seconds
                .cmp(&a.seconds)
                .then_with(|| a.label.cmp(&b.label))
        });
        Ok(out)
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
    use crate::classify::{Client, DEFAULT_CATEGORIES, Rule, RuleField, Tag};
    use crate::timesheet::{EntryKind, ProjectMapping, Timesheet, TimesheetConfig, TimesheetEntry};

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
            block_from: None,
        }
    }

    #[test]
    fn backups_are_complete_and_can_be_inspected() {
        let dir = std::env::temp_dir().join(format!("tracky-backup-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("kum.db")).unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        let copy = dir.join("yedek.db");
        store.backup_to(&copy).unwrap();
        assert!(
            store.backup_to(&copy).is_err(),
            "var olan dosyanın üzerine yazılmaz"
        );

        let info = Store::inspect_backup(&copy).unwrap();
        assert_eq!(info.sessions, 1);
        assert_eq!(info.last_activity, Some(t(600)));
        let restored = Store::open(&copy).unwrap();
        assert_eq!(restored.device_id(), store.device_id());
        assert_eq!(restored.app_totals(t(0), t(3600)).unwrap().len(), 1);

        let junk = dir.join("not.db");
        std::fs::write(&junk, "merhaba").unwrap();
        assert!(Store::inspect_backup(&junk).is_err());
        let empty = dir.join("bos.db");
        Connection::open(&empty).unwrap();
        assert!(Store::inspect_backup(&empty).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_database_includes_wal_contents() {
        let dir = std::env::temp_dir().join(format!("tracky-walcopy-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("kum.db")).unwrap();
        store
            .conn
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        // Açık veritabanının dosyaları kopyalanır: son oturum yalnızca -wal'da.
        let moved = dir.join("eski");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::copy(dir.join("kum.db"), moved.join("kum.db")).unwrap();
        std::fs::copy(dir.join("kum.db-wal"), moved.join("kum.db-wal")).unwrap();
        drop(store);

        let target = dir.join("kopya.db");
        Store::copy_database(&moved.join("kum.db"), &target).unwrap();
        assert_eq!(Store::inspect_backup(&target).unwrap().sessions, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_file_folds_wal_into_the_database() {
        let dir = std::env::temp_dir().join(format!("tracky-ckpt-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kum.db");
        let store = Store::open(&path).unwrap();
        store
            .conn
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        // Çökme gibi: bağlantı kapanmadan dosyalar kenara alınır.
        let crashed = dir.join("cokme.db");
        std::fs::copy(&path, &crashed).unwrap();
        std::fs::copy(dir.join("kum.db-wal"), dir.join("cokme.db-wal")).unwrap();
        drop(store);
        Store::checkpoint_file(&crashed).unwrap();
        let wal = std::fs::metadata(dir.join("cokme.db-wal")).map_or(0, |m| m.len());
        assert_eq!(wal, 0);
        let alone = dir.join("yalniz.db");
        std::fs::copy(&crashed, &alone).unwrap();
        assert_eq!(Store::inspect_backup(&alone).unwrap().sessions, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restored_rows_are_newer_and_unsynced() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        store
            .conn
            .execute_batch(
                "UPDATE sessions SET synced_at = updated_at;
                 UPDATE tags SET synced_at = updated_at;",
            )
            .unwrap();
        store
            .save_setting("sync_cursor:sessions", &"2026-01-01T00:00:00Z")
            .unwrap();
        let before = Utc::now().timestamp_millis();
        store.mark_restored_for_sync().unwrap();
        let (pending, oldest): (i64, i64) = store
            .conn
            .query_row(
                "SELECT COUNT(*), MIN(updated_at) FROM sessions WHERE synced_at IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pending, 1);
        assert!(oldest >= before);
        let synced_tags: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE synced_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(synced_tags, 0);
        assert!(
            store
                .setting::<String>("sync_cursor:sessions")
                .unwrap()
                .is_none()
        );
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
        // Başka uygulama sayılmaz (çakışan oturum başka cihaz demek olurdu, bu yüzden sonra).
        for s in [&a, &b, &c, &session("Code", None, 150, 500)] {
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
    fn title_totals_count_device_overlaps_once() {
        let store = Store::open_in_memory().unwrap();
        let mut a = session("Safari", None, 0, 60);
        a.title = "GitHub".into();
        let mut b = session("Safari", None, 30, 90);
        b.title = "GitHub".into();
        store.upsert_session(&a).unwrap();
        store.upsert_session(&b).unwrap();
        // İkinci oturum başka bilgisayardan: 30–60 arası iki kez sayılmamalı.
        store
            .conn()
            .execute(
                "UPDATE sessions SET device_id = ?1 WHERE id = ?2",
                params![Uuid::new_v4().to_string(), b.id.to_string()],
            )
            .unwrap();
        let got = store
            .title_totals("com.test.Safari", t(0), t(1000))
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].seconds, 90);
    }

    #[test]
    fn tag_totals_sum_milliseconds_before_rounding() {
        let store = Store::open_in_memory().unwrap();
        let dev = store
            .tags()
            .unwrap()
            .into_iter()
            .find(|t| t.name == "Geliştirme")
            .unwrap();
        // Üç 1,5 saniyelik oturum 4,5 saniyedir (→ 4); tek tek kırpılsa 3 olurdu.
        for i in 0..3 {
            let mut s = session("Code", None, i * 10, i * 10);
            s.app_id = "com.microsoft.VSCode".into();
            s.ended_at = s.started_at + chrono::Duration::milliseconds(1500);
            store.upsert_session(&s).unwrap();
        }
        let totals = store.category_totals(t(0), t(1000)).unwrap();
        assert_eq!(totals.get(&dev.id), Some(&4));
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
    fn edits_refuse_another_devices_live_session() {
        let now = Utc::now();
        let mut live = session("Code", None, 0, 0);
        live.started_at = now - chrono::Duration::hours(1);
        live.ended_at = now;
        let (from, to) = (
            now - chrono::Duration::minutes(30),
            now - chrono::Duration::minutes(20),
        );

        // Kendi cihazının süren oturumu düzenlenebilir.
        let own = Store::open_in_memory().unwrap();
        own.upsert_session(&live).unwrap();
        assert_eq!(own.delete_between(from, to).unwrap(), 1);

        // Başka cihazınki reddedilir ve hiçbir şey bölünmez.
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&live).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET device_id = ?1",
                params![Uuid::new_v4().to_string()],
            )
            .unwrap();
        for result in [
            store.set_category_between(from, to, None),
            store.set_project_between(from, to, None),
            store.delete_between(from, to),
        ] {
            assert!(matches!(result, Err(StoreError::ForeignLiveSession)));
        }
        assert_eq!(store.sessions_between(from, to).unwrap().len(), 1);

        // Bittikten (pencere geçtikten) sonra düzenlenebilir.
        let ended = ms(now) - FOREIGN_LIVE_WINDOW_MS - 1;
        store
            .conn()
            .execute("UPDATE sessions SET ended_at = ?1", params![ended])
            .unwrap();
        assert_eq!(store.delete_between(from, to).unwrap(), 1);
    }

    #[test]
    fn assign_app_category_keeps_prefix_rules_for_other_apps() {
        let store = Store::open_in_memory().unwrap();
        let tags = store.tags().unwrap();
        let comm = tags.iter().find(|t| t.name == "İletişim").unwrap();
        let dev = tags.iter().find(|t| t.name == "Geliştirme").unwrap();
        store
            .assign_app_category("com.jetbrains.pycharm", Some(&comm.id))
            .unwrap();
        let classifier = Classifier::new(&store.tags().unwrap(), &store.rules().unwrap());
        assert_eq!(
            classifier.app_category("com.jetbrains.pycharm").as_deref(),
            Some(comm.id.as_str())
        );
        assert_eq!(
            classifier.app_category("com.jetbrains.intellij").as_deref(),
            Some(dev.id.as_str())
        );
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
    fn timesheet_rows_can_be_edited_added_and_exported() {
        let store = Store::open_in_memory().unwrap();
        let mut work = session("Figma", None, 0, 3600);
        work.title = "Trumore Loyalty UI/UX — Figma".into();
        store.upsert_session(&work).unwrap();
        let p = store.accept_project_suggestion("Trumore").unwrap();
        let sheet = Timesheet {
            id: "togg".into(),
            default_party: "ADBA".into(),
            projects: vec![ProjectMapping {
                project_id: p.id.clone(),
                division: String::new(),
                party: None,
                default_details: None,
            }],
            ..Default::default()
        };
        store
            .save_timesheet_config(&TimesheetConfig {
                timesheets: vec![sheet.clone()],
                ..Default::default()
            })
            .unwrap();
        let ctx = store.timesheet_context().unwrap();
        let (from, to) = (t(-36_000), t(36_000));
        let pieces = store.timesheet_pieces(&ctx, from, to, &[]).unwrap();
        let day = chrono::DateTime::<chrono::Local>::from(t(0)).date_naive();
        let rows = |store: &Store| {
            let pieces = store.timesheet_pieces(&ctx, from, to, &[]).unwrap();
            store
                .timesheet_day(&ctx, &sheet, day, &pieces)
                .unwrap()
                .rows
        };
        let proposed = store
            .timesheet_day(&ctx, &sheet, day, &pieces)
            .unwrap()
            .rows;
        assert_eq!(proposed.len(), 1);
        assert_eq!(
            (
                proposed[0].entry.division.as_str(),
                proposed[0].entry.party.as_str()
            ),
            ("Trumore", "ADBA")
        );
        assert_eq!(proposed[0].entry.project_id, p.id);
        assert_eq!(proposed[0].id, None, "düzenlenene kadar canlı öneri");

        // Düzenle (kaydedilir), elle satır ekle.
        let mut edited = proposed[0].entry.clone();
        edited.details = "Loyalty ekranları".into();
        let id = store.save_timesheet_entry(None, &edited).unwrap();
        let mut extra = edited.clone();
        extra.kind = EntryKind::F2F;
        extra.hours = 0.5;
        extra.coverage = None;
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

        // Aktarılan kayıt korunur: değiştirilemez, gün yeniden önerilince silinmez ve işi
        // ikinci kez önerilmez (dosyaya iki kez yazılırdı).
        store
            .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), &sheet.id, "")
            .unwrap();
        assert!(store.save_timesheet_entry(Some(&id), &edited).is_err());
        assert_eq!(store.reset_timesheet_day(&sheet, day).unwrap(), 1);
        let left = rows(&store);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].entry.details, "Loyalty ekranları");
        assert!(left[0].exported);
        assert!(
            left.iter()
                .all(|e| e.id.as_deref() != Some(extra_id.as_str()))
        );
    }

    #[test]
    fn report_counts_overlapping_devices_once() {
        let store = Store::open_in_memory().unwrap();
        // İki bilgisayar: 0–3600 dizüstü, 1800–2400 masaüstü (eşitlemeyle gelmiş gibi).
        store
            .upsert_session(&session("Video", None, 0, 3600))
            .unwrap();
        store
            .upsert_session(&session("Figma", None, 1800, 2400))
            .unwrap();
        let r = store.report(t(0), t(3600), &[t(0)], false).unwrap();
        assert_eq!(r.total_seconds, 3600);
        let figma = r.apps.iter().find(|a| a.app_name == "Figma").unwrap();
        assert_eq!(figma.seconds, 600);
        // Ham kayıtlar değişmez.
        assert_eq!(store.sessions_between(t(0), t(3600)).unwrap().len(), 2);
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
    fn setting_updated_at_never_goes_backwards() {
        let store = Store::open_in_memory().unwrap();
        store.save_setting("theme", &"dark").unwrap();
        // Saati ileride olan bir cihazdan gelmiş sürüm.
        let future = ms(Utc::now()) + 3_600_000;
        store
            .conn
            .execute(
                "UPDATE settings SET updated_at = ?1, synced_at = ?1 WHERE key = 'theme'",
                [future],
            )
            .unwrap();
        store.save_setting("theme", &"light").unwrap();
        let after: i64 = store
            .conn
            .query_row(
                "SELECT updated_at FROM settings WHERE key = 'theme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(after > future);
    }

    #[test]
    fn empty_or_reversed_ranges_are_rejected() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        assert!(matches!(
            store.delete_between(t(300), t(300)),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            store.delete_between(t(300), t(240)),
            Err(StoreError::Invalid(_))
        ));
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
    fn unassigned_block_adopts_project_it_grows_over() {
        let store = Store::open_in_memory().unwrap();
        let project = store.accept_project_suggestion("Togg").unwrap();
        let p = Some(project.id.as_str());
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        store
            .upsert_session(&session("B", None, 600, 1200))
            .unwrap();
        store.set_project_between(t(0), t(600), p).unwrap();

        // Projesiz B bloğunun başı projeli A'nın üstüne uzar: ikisi de Togg olur.
        store
            .resize_block(t(600), t(1200), t(0), t(1200), "B", None, None)
            .unwrap();
        let all = store.sessions_between(t(0), t(1200)).unwrap();
        assert!(all.iter().all(|s| s.project_id.as_deref() == p));
        let report = store.report(t(0), t(1200), &[t(0)], true).unwrap();
        assert_eq!(report.work.blocks.len(), 1);
        assert_eq!(report.work.blocks[0].project_id.as_deref(), p);
    }

    #[test]
    fn resize_block_trims_joins_and_fills_gaps() {
        let store = Store::open_in_memory().unwrap();
        let cat = store.tags().unwrap()[0].id.clone();
        let project = store.accept_project_suggestion("Togg").unwrap();
        // Blok 600–1800; önünde başka iş, arkasında boşta süre ve kaydı olmayan boşluk.
        store.upsert_session(&session("B", None, 0, 600)).unwrap();
        store
            .upsert_session(&session("A", None, 600, 1800))
            .unwrap();
        store
            .upsert_session(&Session::idle(t(1800), t(2400)))
            .unwrap();
        let p = Some(project.id.as_str());
        store.set_project_between(t(600), t(1800), p).unwrap();
        store
            .set_category_between(t(600), t(1800), Some(&cat))
            .unwrap();

        // Sonu 3000'e uzar, başı 900'e kısalır.
        store
            .resize_block(t(600), t(1800), t(900), t(3000), "Togg", Some(&cat), p)
            .unwrap();
        let all = store.sessions_between(t(0), t(3600)).unwrap();
        let work: Vec<_> = all.iter().filter(|s| s.counts_as_work()).collect();
        // Kısalan kısım silinmedi: projesiyle ayrı blok oldu; önceki iş yerinde.
        assert!(all.iter().any(|s| s.app_name == "A"
            && (s.started_at, s.ended_at) == (t(600), t(900))
            && s.project_id.as_deref() == p
            && s.block_from.is_none()));
        assert!(
            all.iter()
                .any(|s| s.started_at == t(900) && s.block_from == Some(t(900)))
        );
        let report = store.report(t(0), t(3600), &[t(0)], true).unwrap();
        let spans: Vec<_> = report
            .work
            .blocks
            .iter()
            .filter(|b| b.project_id.as_deref() == p)
            .map(|b| (b.start, b.end))
            .collect();
        assert_eq!(spans, [(t(600), t(900)), (t(900), t(3000))]);
        assert!(
            work.iter()
                .any(|s| s.app_name == "B" && s.ended_at == t(600))
        );
        // Uzayan kısım: boşta süre atanınca çalışma olur, kaydı olmayan boşluk elle kayıtla
        // dolar; hepsi projede ve kategoride.
        let joined: Vec<_> = work.iter().filter(|s| s.started_at >= t(1800)).collect();
        assert_eq!(joined.len(), 2);
        assert!(joined[0].is_idle() && joined[0].ended_at == t(2400));
        assert!(joined[1].is_manual());
        assert_eq!(
            (joined[1].started_at, joined[1].ended_at),
            (t(2400), t(3000))
        );
        let block: i64 = work
            .iter()
            .filter(|s| s.started_at >= t(900))
            .inspect(|s| {
                assert_eq!(s.project_id.as_deref(), p);
                assert_eq!(s.category_id.as_deref(), Some(&*cat));
            })
            .map(|s| (s.ended_at - s.started_at).num_seconds())
            .sum();
        assert_eq!(block, 2100);

        // Başı başka işin üstüne uzayınca o iş ve kesilen parça bloğa katılır: bölme kalkar.
        store
            .resize_block(t(900), t(3000), t(300), t(3000), "Togg", Some(&cat), p)
            .unwrap();
        let b = store.sessions_between(t(300), t(600)).unwrap();
        assert!(
            b.iter()
                .all(|s| s.app_name == "B" && s.project_id.as_deref() == p)
        );
        let all = store.sessions_between(t(0), t(3600)).unwrap();
        assert!(all.iter().all(|s| s.block_from.is_none()));
        let report = store.report(t(0), t(3600), &[t(0)], true).unwrap();
        assert!(
            report
                .work
                .blocks
                .iter()
                .any(|b| (b.start, b.end) == (t(300), t(3000)))
        );

        // Gelecek ve ters aralık reddedilir.
        let later = Utc::now() + chrono::Duration::hours(1);
        assert!(
            store
                .resize_block(t(300), t(3000), t(300), later, "Togg", None, p)
                .is_err()
        );
        assert!(
            store
                .resize_block(t(300), t(3000), t(3000), t(300), "Togg", None, p)
                .is_err()
        );
    }

    #[test]
    fn scoped_range_edits_touch_only_the_chosen_app_or_window() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        store.upsert_session(&session("B", None, 0, 600)).unwrap();
        store
            .upsert_session(&Session {
                title: "u".into(),
                ..session("A", None, 600, 900)
            })
            .unwrap();
        let project = store.accept_project_suggestion("Togg").unwrap();
        let only_a = EditScope {
            app_ids: vec!["com.test.A".into()],
            titles: None,
        };
        // A'nın iki penceresi projeye geçer, aynı dilimdeki B değişmez.
        assert_eq!(
            store
                .set_project_in(t(0), t(900), Some(&project.id), Some(&only_a))
                .unwrap(),
            2
        );
        let projects = |store: &Store| {
            let mut v: Vec<(String, String, Option<String>)> = store
                .sessions_between(t(0), t(3600))
                .unwrap()
                .into_iter()
                .map(|s| (s.app_name, s.title, s.project_id))
                .collect();
            v.sort();
            v
        };
        let p = Some(project.id.clone());
        assert_eq!(
            projects(&store),
            [
                ("A".into(), "t".into(), p.clone()),
                ("A".into(), "u".into(), p.clone()),
                ("B".into(), "t".into(), None),
            ]
        );
        // Yalnızca bir pencere silinir.
        let window = EditScope {
            app_ids: vec!["com.test.A".into()],
            titles: Some(vec!["u".into()]),
        };
        assert_eq!(store.delete_in(t(0), t(900), Some(&window)).unwrap(), 1);
        assert_eq!(projects(&store).len(), 2);
        // Boş kapsam reddedilir (hiçbir şeye dokunmaz değil, hata).
        assert!(
            store
                .delete_in(t(0), t(900), Some(&EditScope::default()))
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
    fn idle_time_is_left_out_of_totals_until_assigned() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("A", None, 0, 600)).unwrap();
        store
            .upsert_session(&Session::idle(t(600), t(2400)))
            .unwrap();
        let (from, to) = (t(0), t(3600));
        assert_eq!(
            store.report(from, to, &[from], true).unwrap().total_seconds,
            600
        );
        assert_eq!(store.app_totals(from, to).unwrap().len(), 1);
        assert!(
            store
                .known_apps(10)
                .unwrap()
                .iter()
                .all(|a| a.key != IDLE_APP_ID)
        );
        assert!(
            !store
                .export_csv()
                .unwrap()
                .contains(crate::model::IDLE_NAME)
        );

        // Bir kısmı projeye atanınca o kısım çalışma olur, kalanı boşta kalır.
        let project = store.accept_project_suggestion("Togg").unwrap();
        store
            .set_project_between(t(600), t(1200), Some(&project.id))
            .unwrap();
        let r = store.report(from, to, &[from], true).unwrap();
        assert_eq!(r.total_seconds, 1200);
        assert_eq!(r.idle_seconds, 1200);
        assert_eq!(store.project_totals(from, to).unwrap()[&project.id], 600);
    }

    #[test]
    fn manual_entry_replaces_idle_time() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&Session::idle(t(0), t(3600))).unwrap();
        store
            .add_manual_session("Toplantı", t(600), t(1800), None, None)
            .unwrap();
        let all = store.sessions_between(t(0), t(3600)).unwrap();
        let spans: Vec<_> = all
            .iter()
            .map(|s| (s.is_idle(), s.started_at, s.ended_at))
            .collect();
        assert_eq!(
            spans,
            [
                (true, t(0), t(600)),
                (false, t(600), t(1800)),
                (true, t(1800), t(3600))
            ]
        );
        // Projeye atanmış boşta süre ise elle kaydın üstüne yazılmaz.
        let project = store.accept_project_suggestion("Togg").unwrap();
        store
            .set_project_between(t(0), t(600), Some(&project.id))
            .unwrap();
        assert!(
            store
                .add_manual_session("Okuma", t(0), t(300), None, None)
                .is_err()
        );
    }

    #[test]
    fn domain_rules_are_normalized_and_classify_by_url() {
        let store = Store::open_in_memory().unwrap();
        let project = store.accept_project_suggestion("Togg").unwrap();
        let rule = |pattern: &str| Rule {
            id: Uuid::new_v4().to_string(),
            tag_id: project.id.clone(),
            field: RuleField::Domain,
            pattern: pattern.into(),
        };
        store
            .upsert_rule(&rule("https://www.Jira.Togg.com/"))
            .unwrap();
        assert!(store.upsert_rule(&rule("toplantı notları")).is_err());
        assert!(
            store
                .rules()
                .unwrap()
                .iter()
                .any(|r| r.field == RuleField::Domain && r.pattern == "jira.togg.com")
        );
        store
            .upsert_session(&session(
                "Safari",
                Some("https://jira.togg.com/browse/T-1"),
                0,
                600,
            ))
            .unwrap();
        assert_eq!(
            store.project_totals(t(0), t(3600)).unwrap()[&project.id],
            600
        );
    }

    #[test]
    fn unknown_rule_fields_from_newer_versions_are_skipped() {
        let store = Store::open_in_memory().unwrap();
        let before = store.rules().unwrap().len();
        let tag = store.tags().unwrap()[0].id.clone();
        // Bu sürümün tanımadığı bir tür (CHECK'i atlatmak için doğrudan yazılır).
        store
            .conn()
            .execute_batch(&format!(
                "PRAGMA ignore_check_constraints = ON;
                 INSERT INTO rules (id, tag_id, field, pattern, updated_at)
                 VALUES ('x', '{tag}', 'gelecek', 'p', 0);
                 PRAGMA ignore_check_constraints = OFF;"
            ))
            .unwrap();
        assert_eq!(store.rules().unwrap().len(), before);
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
