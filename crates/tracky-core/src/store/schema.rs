//! Şema göçleri: sırasıyla uygulanır, indeks + 1 = `PRAGMA user_version`.

use rusqlite::Connection;

use super::{Result, StoreError};

/// Sırasıyla uygulanan şema göçleri; indeks + 1 = `PRAGMA user_version`.
///
/// Senkronizasyona hazırlık: tüm satırlar UUID ile tanımlanır, her satırda
/// `device_id`, `updated_at` (son değişiklik) ve `deleted_at` (yumuşak silme)
/// bulunur; `synced_at` yalnızca yerelde tutulur.
pub(super) const MIGRATIONS: &[&str] = &[
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

pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
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
