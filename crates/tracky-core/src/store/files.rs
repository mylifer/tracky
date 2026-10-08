//! Veritabanı dosyası: yedek alma, yedeği inceleme, bütünlük denetimi ve geri yükleme.

use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::schema::MIGRATIONS;
use super::{Result, Store, StoreError, from_ms, ms};

/// Yedek dosyasının içeriği (geri yüklemeden önce göstermek için).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub sessions: i64,
    pub last_activity: Option<DateTime<Utc>>,
}

impl Store {
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

    /// Veritabanı dosyasının sayfa ve dizin bütünlüğü (`PRAGMA quick_check`), ayrı salt okunur
    /// bağlantıyla: takibi kilitlemez. Sağlamsa `None`, değilse ilk sorunlar.
    pub fn check_file(path: &Path) -> Result<Option<String>> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut stmt = conn.prepare("PRAGMA quick_check(5)")?;
        let problems: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok((problems != ["ok"]).then(|| problems.join("; ")))
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
}
