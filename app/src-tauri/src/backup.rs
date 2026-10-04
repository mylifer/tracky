//! Veritabanı yedekleri: haftada bir kendiliğinden ya da istenince; yedekten geri yükleme.
//!
//! Yedekler uygulama veri klasöründeki `backups/` altında `kum-YYYY-MM-DD-HHMM.db` adıyla
//! durur, en yeni `KEEP` tanesi saklanır. Geri yükleme açık veritabanını değiştiremez: seçilen
//! yedek `kum.db.restore` olarak kopyalanır, uygulama yeniden başlar ve açılışta
//! ([`apply_pending_restore`]) eski veritabanı `backups/` altına alınıp yedek yerine konur.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::{BackupInfo, Store};

use crate::lock;
use crate::tracking::Shared;

type CmdResult<T> = Result<T, String>;

const DIR: &str = "backups";
const PREFIX: &str = "kum-";
/// Saklanan en yeni yedek sayısı (haftalıkta iki aya yakın).
const KEEP: usize = 8;
/// Otomatik yedek aralığı.
const EVERY: chrono::Duration = chrono::Duration::days(7);
/// Son yedeğin zamanı.
const LAST_KEY: &str = "last_backup_at";
/// Yeniden başlayınca yerine konacak yedek.
const RESTORE_FILE: &str = "kum.db.restore";
pub const DB_FILE: &str = "kum.db";
/// Açılıştan sonra ilk denetim (açılışı yavaşlatmasın) ve sonraki denetimlerin aralığı.
const FIRST_CHECK: Duration = Duration::from_secs(120);
const CHECK_EVERY: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub name: String,
    pub path: String,
    pub at: DateTime<Utc>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub dir: String,
    pub last: Option<DateTime<Utc>>,
    /// En yeniden eskiye.
    pub files: Vec<BackupFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedBackup {
    pub path: String,
    #[serde(flatten)]
    pub info: BackupInfo,
}

fn data_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

fn backup_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// Açılışta, veritabanı açılmadan önce: bekleyen geri yükleme varsa uygular. Eski veritabanı
/// (WAL dosyalarıyla) `backups/` altına "geri-yukleme-oncesi" adıyla taşınır, silinmez.
pub fn apply_pending_restore(data_dir: &Path) -> std::io::Result<()> {
    let pending = data_dir.join(RESTORE_FILE);
    if !pending.exists() {
        return Ok(());
    }
    let dir = backup_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let stamp = Local::now().format("%Y-%m-%d-%H%M%S");
    for suffix in ["", "-wal", "-shm"] {
        let current = data_dir.join(format!("{DB_FILE}{suffix}"));
        if current.exists() {
            std::fs::rename(
                &current,
                dir.join(format!("geri-yukleme-oncesi-{stamp}.db{suffix}")),
            )?;
        }
    }
    std::fs::rename(pending, data_dir.join(DB_FILE))
}

/// Yedeği alır, eskileri temizler ve zamanı kaydeder.
fn backup_now_inner(app: &AppHandle) -> CmdResult<BackupFile> {
    let dir = backup_dir(&data_dir(app)?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let now = Utc::now();
    let mut path = dir.join(format!(
        "{PREFIX}{}.db",
        now.with_timezone(&Local).format("%Y-%m-%d-%H%M")
    ));
    // Aynı dakikada ikinci yedek.
    if path.exists() {
        path = dir.join(format!(
            "{PREFIX}{}.db",
            now.with_timezone(&Local).format("%Y-%m-%d-%H%M%S")
        ));
    }
    {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        store.backup_to(&path).map_err(|e| e.to_string())?;
        store
            .save_setting(LAST_KEY, &now)
            .map_err(|e| e.to_string())?;
    }
    prune(&dir);
    file_info(&path).ok_or_else(|| "yedek dosyası okunamadı".to_string())
}

/// En yeni `KEEP` yedek kalır; geri yükleme öncesi kopyalara dokunulmaz.
fn prune(dir: &Path) {
    let mut files = list(dir);
    for old in files.split_off(KEEP.min(files.len())) {
        let _ = std::fs::remove_file(old.path);
    }
}

/// Klasördeki yedekler, en yeniden eskiye.
fn list(dir: &Path) -> Vec<BackupFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<BackupFile> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(".db"))
        })
        .filter_map(|p| file_info(&p))
        .collect();
    files.sort_by(|a, b| b.at.cmp(&a.at).then(b.name.cmp(&a.name)));
    files
}

fn file_info(path: &Path) -> Option<BackupFile> {
    let meta = std::fs::metadata(path).ok()?;
    Some(BackupFile {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: path.display().to_string(),
        at: meta.modified().ok()?.into(),
        bytes: meta.len(),
    })
}

fn last_backup(app: &AppHandle) -> Option<DateTime<Utc>> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    store.setting(LAST_KEY).ok().flatten()
}

/// Arka planda saatte bir: son yedek bir haftadan eskiyse yenisini alır.
pub fn start(app: &tauri::App) -> std::io::Result<()> {
    let app = app.handle().clone();
    std::thread::Builder::new()
        .name("kum-backup".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                let due = last_backup(&app).is_none_or(|at| Utc::now() - at >= EVERY);
                if due && let Err(e) = backup_now_inner(&app) {
                    eprintln!("yedek alınamadı: {e}");
                }
                std::thread::sleep(CHECK_EVERY);
            }
        })?;
    Ok(())
}

#[tauri::command]
pub async fn backup_status(app: AppHandle) -> CmdResult<BackupStatus> {
    let dir = backup_dir(&data_dir(&app)?);
    Ok(BackupStatus {
        dir: dir.display().to_string(),
        last: last_backup(&app),
        files: list(&dir),
    })
}

#[tauri::command]
pub async fn backup_now(app: AppHandle) -> CmdResult<BackupFile> {
    backup_now_inner(&app)
}

/// Yedek klasörünü Finder / Gezgin'de açar.
#[tauri::command]
pub async fn open_backup_folder(app: AppHandle) -> CmdResult<()> {
    let dir = backup_dir(&data_dir(&app)?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(windows)]
    let opener = "explorer";
    #[cfg(not(any(target_os = "macos", windows)))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Yedek dosyası seçtirir ve içeriğini gösterir (henüz geri yüklemez); vazgeçilirse `None`.
#[tauri::command]
pub async fn pick_backup(app: AppHandle) -> CmdResult<Option<PickedBackup>> {
    let dir = backup_dir(&data_dir(&app)?);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Geri yüklenecek Kum yedeği")
            .set_directory(dir)
            .add_filter("Kum yedeği", &["db"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    let info = Store::inspect_backup(&path).map_err(|e| e.to_string())?;
    Ok(Some(PickedBackup {
        path: path.display().to_string(),
        info,
    }))
}

/// Yedeği geri yükler: dosyayı doğrular, açılışta yerine konmak üzere kopyalar ve uygulamayı
/// yeniden başlatır. Şimdiki veritabanı silinmez, yedek klasörüne taşınır.
#[tauri::command]
pub async fn restore_backup(app: AppHandle, path: String) -> CmdResult<()> {
    let source = PathBuf::from(&path);
    Store::inspect_backup(&source).map_err(|e| e.to_string())?;
    let target = data_dir(&app)?.join(RESTORE_FILE);
    std::fs::copy(&source, &target).map_err(|e| e.to_string())?;
    // `request_restart` olağan kapanıştan geçer: takip süren oturumu yazar (bkz. updater).
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_restore_replaces_the_database_and_keeps_the_old_one() {
        let dir = std::env::temp_dir().join(format!("kum-restore-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DB_FILE), "eski").unwrap();
        std::fs::write(dir.join(format!("{DB_FILE}-wal")), "eski-wal").unwrap();
        // Bekleyen geri yükleme yokken bir şey değişmez.
        apply_pending_restore(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(DB_FILE)).unwrap(), "eski");

        std::fs::write(dir.join(RESTORE_FILE), "yedek").unwrap();
        apply_pending_restore(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(DB_FILE)).unwrap(), "yedek");
        assert!(!dir.join(RESTORE_FILE).exists());
        assert!(!dir.join(format!("{DB_FILE}-wal")).exists());
        let kept: Vec<String> = std::fs::read_dir(backup_dir(&dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept.iter().all(|n| n.starts_with("geri-yukleme-oncesi-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_the_newest_backups_only() {
        let dir = std::env::temp_dir().join(format!("kum-prune-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 3 {
            let path = dir.join(format!("{PREFIX}2026-01-{:02}-0900.db", i + 1));
            std::fs::write(&path, "x").unwrap();
            let at = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        std::fs::write(dir.join("geri-yukleme-oncesi-x.db"), "x").unwrap();
        prune(&dir);
        let left = list(&dir);
        assert_eq!(left.len(), KEEP);
        assert_eq!(
            left[0].name,
            format!("{PREFIX}2026-01-{:02}-0900.db", KEEP + 3)
        );
        assert!(dir.join("geri-yukleme-oncesi-x.db").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
