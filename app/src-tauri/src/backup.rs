//! Veritabanı yedekleri: haftada bir kendiliğinden ya da istenince; yedekten geri yükleme.
//!
//! Yedekler uygulama veri klasöründeki `backups/` altında `kum-YYYY-MM-DD-HHMM.db` adıyla
//! durur, en yeni `KEEP` tanesi saklanır. Geri yükleme açık veritabanını değiştiremez: seçilen
//! yedek `kum.db.restore` olarak kopyalanır, uygulama yeniden başlar ve açılışta
//! ([`apply_pending_restore`]) eski veritabanı `backups/` altına alınıp yedek yerine konur.
//! Geri yüklenen veritabanı açılınca ([`finish_restore`]) eşitleme durumu sıfırlanır: yedekteki
//! eski imleçler ve "gönderildi" işaretleri sunucudaki yeni sürümlerin geri yüklemeyi ezmesine
//! yol açardı.

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
/// Yedek yerine kondu, eşitleme durumu henüz sıfırlanmadı ([`finish_restore`]).
const RESTORED_MARK: &str = "kum.db.restored";
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
/// Bir adım başarısız olursa yapılan taşımalar geri alınır: veritabanı yerinde kalır, yanında
/// sahipsiz bir `-wal` kalıp yeni açılan veritabanını bozmaz.
pub fn apply_pending_restore(data_dir: &Path) -> std::io::Result<()> {
    let pending = data_dir.join(RESTORE_FILE);
    if !pending.exists() {
        return Ok(());
    }
    let dir = backup_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let stamp = Local::now().format("%Y-%m-%d-%H%M%S");
    // Açılamayan bir dosya yerine konursa uygulama her açılışta çökerdi: şimdiki veritabanı
    // yerinde kalır, dosya incelenmek üzere kenara alınır.
    if let Err(e) = Store::inspect_backup(&pending) {
        std::fs::rename(
            &pending,
            dir.join(format!("gecersiz-geri-yukleme-{stamp}.db")),
        )?;
        return Err(std::io::Error::other(e.to_string()));
    }
    let db = data_dir.join(DB_FILE);
    // Kenara alınan kopya `-wal` olmadan da eksiksiz olsun (çökmeden kalan WAL). Olmazsa
    // WAL dosyaları yine birlikte taşınır.
    if db.exists()
        && let Err(e) = Store::checkpoint_file(&db)
    {
        eprintln!("veritabanı WAL'ı işlenemedi: {e}");
    }
    let mark = data_dir.join(RESTORED_MARK);
    // Damga, geri yüklenen veritabanı açılamazsa eskisine dönmek için ([`undo_restore`]).
    std::fs::write(&mark, stamp.to_string())?;
    // Önce WAL dosyaları: ana dosya taşınıp WAL kalırsa, açılışta yeni veritabanı eskinin
    // WAL'ıyla açılırdı. En son yedek yerine konur.
    let mut moves: Vec<(PathBuf, PathBuf)> = ["-wal", "-shm", ""]
        .into_iter()
        .map(|suffix| {
            (
                data_dir.join(format!("{DB_FILE}{suffix}")),
                dir.join(format!("geri-yukleme-oncesi-{stamp}.db{suffix}")),
            )
        })
        .filter(|(current, _)| current.exists())
        .collect();
    moves.push((pending.clone(), db));
    // Yeniden başlatmada eski süreç dosyayı bir an daha tutabilir (Windows'ta açık dosya
    // taşınamaz): kısa aralıklarla yeniden denenir.
    let mut tries = 0;
    loop {
        match move_all(&moves) {
            Ok(()) => return Ok(()),
            Err(_) if tries < 5 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&mark);
                // Bekleyen geri yükleme kenara alınır: günler sonraki bir açılışta habersizce
                // uygulanıp aradaki kayıtları kenara taşımasın.
                let _ = std::fs::rename(
                    &pending,
                    dir.join(format!("uygulanamayan-geri-yukleme-{stamp}.db")),
                );
                return Err(e);
            }
        }
    }
}

/// Geri yüklenen veritabanı açılamadıysa (örn. geçiş hatası): onu kenara alır ve geri
/// yüklemeden önceki veritabanını yerine koyar. Geri döndüyse `true`.
pub fn undo_restore(data_dir: &Path) -> std::io::Result<bool> {
    let mark = data_dir.join(RESTORED_MARK);
    let Ok(stamp) = std::fs::read_to_string(&mark) else {
        return Ok(false);
    };
    let stamp = stamp.trim();
    let dir = backup_dir(data_dir);
    let before = dir.join(format!("geri-yukleme-oncesi-{stamp}.db"));
    if stamp.is_empty() || !before.exists() {
        return Ok(false);
    }
    let mut moves: Vec<(PathBuf, PathBuf)> = ["-wal", "-shm", ""]
        .into_iter()
        .map(|suffix| {
            (
                data_dir.join(format!("{DB_FILE}{suffix}")),
                dir.join(format!("acilamayan-geri-yukleme-{stamp}.db{suffix}")),
            )
        })
        .filter(|(current, _)| current.exists())
        .collect();
    moves.extend(["-wal", "-shm", ""].into_iter().filter_map(|suffix| {
        let from = dir.join(format!("geri-yukleme-oncesi-{stamp}.db{suffix}"));
        from.exists()
            .then(|| (from, data_dir.join(format!("{DB_FILE}{suffix}"))))
    }));
    move_all(&moves)?;
    std::fs::remove_file(&mark)?;
    Ok(true)
}

/// Dosyaları sırayla taşır; biri başarısız olursa öncekileri geri taşır.
fn move_all(moves: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
    for (i, (from, to)) in moves.iter().enumerate() {
        if let Err(e) = std::fs::rename(from, to) {
            for (from, to) in moves[..i].iter().rev() {
                if let Err(e) = std::fs::rename(to, from) {
                    eprintln!("{} geri taşınamadı: {e}", from.display());
                }
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Geri yükleme uygulandıysa, veritabanı açıldıktan sonra (eşitleme başlamadan) çağrılır:
/// eşitleme imleçlerini ve oturumunu sıfırlar, geri yüklenen satırları yeniden gönderilecek
/// ve sunucudakilerden yeni işaretler (bkz. [`Store::mark_restored_for_sync`]). Kullanıcı
/// eşitleme için yeniden giriş yapar: yedekteki yenileme jetonu çoktan değişmiş olabilir.
pub fn finish_restore(data_dir: &Path, store: &Store) -> Result<(), Box<dyn std::error::Error>> {
    let mark = data_dir.join(RESTORED_MARK);
    if !mark.exists() {
        return Ok(());
    }
    // Önce oturum: sonraki adım başarısız olsa da eski durumla eşitlenmez (işaret kaldığı
    // için bir sonraki açılışta yeniden denenir).
    crate::sync::forget_session(store)?;
    store.mark_restored_for_sync()?;
    std::fs::remove_file(mark)?;
    Ok(())
}

/// Yedeği alır, eskileri temizler ve zamanı kaydeder.
fn backup_now_inner(app: &AppHandle) -> CmdResult<BackupFile> {
    let data = data_dir(app)?;
    let dir = backup_dir(&data);
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
    // Ayrı bir salt okunur bağlantıyla: büyük veritabanında kopya sürerken ortak bağlantı
    // kilitli kalıp takibi ve menü çubuğunu dondurmasın.
    Store::copy_database(&data.join(DB_FILE), &path).map_err(|e| e.to_string())?;
    lock(&app.state::<Shared>().store)
        .save_setting(LAST_KEY, &now)
        .map_err(|e| e.to_string())?;
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
    let dir = data_dir(&app)?;
    let target = dir.join(RESTORE_FILE);
    // Önce geçici dosyaya: kopyalama yarıda kalırsa (kapanma, çökme) açılışta yarım bir
    // dosya yerine konmaz.
    let partial = dir.join(format!("{RESTORE_FILE}.tmp"));
    for file in [&partial, &target] {
        if file.exists() {
            std::fs::remove_file(file).map_err(|e| e.to_string())?;
        }
    }
    // Dosya kopyası yanındaki `-wal`'ı (örn. "geri-yukleme-oncesi" kopyalarında) kaybederdi;
    // SQLite ile kopyalanınca WAL'daki değişiklikler de gelir.
    Store::copy_database(&source, &partial).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &target).map_err(|e| e.to_string())?;
    // `request_restart` olağan kapanıştan geçer: takip süren oturumu yazar (bkz. updater).
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_moves_are_rolled_back() {
        let dir = std::env::temp_dir().join(format!("kum-rollback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a-wal"), "wal").unwrap();
        std::fs::write(dir.join("a"), "db").unwrap();
        // Son taşıma (olmayan klasöre) başarısız: öncekiler geri alınır, veritabanı WAL'ıyla
        // yerinde kalır.
        let moves = vec![
            (dir.join("a-wal"), dir.join("b-wal")),
            (dir.join("a"), dir.join("b")),
            (dir.join("yok"), dir.join("yok").join("c")),
        ];
        assert!(move_all(&moves).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(dir.join("a-wal")).unwrap(), "wal");
        assert!(!dir.join("b").exists() && !dir.join("b-wal").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invalid_pending_restore_keeps_the_current_database() {
        let dir = std::env::temp_dir().join(format!("kum-invalid-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DB_FILE), b"current").unwrap();
        std::fs::write(dir.join(RESTORE_FILE), b"yarim kopya").unwrap();
        assert!(apply_pending_restore(&dir).is_err());
        assert_eq!(std::fs::read(dir.join(DB_FILE)).unwrap(), b"current");
        assert!(!dir.join(RESTORE_FILE).exists());
        assert!(!dir.join(RESTORED_MARK).exists());
        let kept = std::fs::read_dir(backup_dir(&dir)).unwrap().count();
        assert_eq!(kept, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restore_that_cannot_open_is_undone() {
        let dir = std::env::temp_dir().join(format!("kum-undo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        drop(Store::open(dir.join(DB_FILE)).unwrap());
        let before = std::fs::read(dir.join(DB_FILE)).unwrap();
        let source = dir.join("yedek.db");
        drop(Store::open(&source).unwrap());
        Store::copy_database(&source, &dir.join(RESTORE_FILE)).unwrap();
        apply_pending_restore(&dir).unwrap();
        assert!(dir.join(RESTORED_MARK).exists());
        assert!(undo_restore(&dir).unwrap());
        assert_eq!(std::fs::read(dir.join(DB_FILE)).unwrap(), before);
        assert!(!dir.join(RESTORED_MARK).exists());
        // İşaret yokken bir şey yapmaz.
        assert!(!undo_restore(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finish_restore_resets_sync_state_once() {
        let dir = std::env::temp_dir().join(format!("kum-finish-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .save_setting(crate::sync::AUTH_KEY, &serde_json::json!({"x": 1}))
            .unwrap();
        // İşaret yokken (olağan açılış) dokunulmaz.
        let signed_in = || {
            store
                .setting::<serde_json::Value>(crate::sync::AUTH_KEY)
                .unwrap()
                .is_some_and(|v| !v.is_null())
        };
        finish_restore(&dir, &store).unwrap();
        assert!(signed_in());
        std::fs::write(dir.join(RESTORED_MARK), b"").unwrap();
        finish_restore(&dir, &store).unwrap();
        assert!(!signed_in());
        assert!(!dir.join(RESTORED_MARK).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_restore_replaces_the_database_and_keeps_the_old_one() {
        let dir = std::env::temp_dir().join(format!("kum-restore-{}", uuid::Uuid::new_v4()));
        let live = dir.join("canli");
        std::fs::create_dir_all(&live).unwrap();
        // Çökmüş gibi: son değişiklik yalnızca -wal'da kalmış veritabanı.
        let store = Store::open(live.join(DB_FILE)).unwrap();
        store.save_setting("eski", &true).unwrap();
        for suffix in ["", "-wal"] {
            std::fs::copy(
                live.join(format!("{DB_FILE}{suffix}")),
                dir.join(format!("{DB_FILE}{suffix}")),
            )
            .unwrap();
        }
        drop(store);
        // Bekleyen geri yükleme yokken bir şey değişmez.
        apply_pending_restore(&dir).unwrap();
        assert!(dir.join(format!("{DB_FILE}-wal")).exists());

        let backup = Store::open_in_memory().unwrap();
        backup.save_setting("yedek", &true).unwrap();
        backup.backup_to(&dir.join(RESTORE_FILE)).unwrap();
        apply_pending_restore(&dir).unwrap();
        assert!(!dir.join(RESTORE_FILE).exists());
        assert!(!dir.join(format!("{DB_FILE}-wal")).exists());
        assert!(dir.join(RESTORED_MARK).exists());
        let restored = Store::open(dir.join(DB_FILE)).unwrap();
        assert_eq!(restored.setting::<bool>("yedek").unwrap(), Some(true));
        assert_eq!(restored.setting::<bool>("eski").unwrap(), None);
        // Eski veritabanı kenarda; WAL'ı işlendiği için -wal olmadan da eksiksiz.
        let kept: Vec<PathBuf> = std::fs::read_dir(backup_dir(&dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert!(kept.iter().all(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("geri-yukleme-oncesi-")
        }));
        let old = kept
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "db"))
            .unwrap();
        let alone = dir.join("yalniz.db");
        std::fs::copy(old, &alone).unwrap();
        assert_eq!(
            Store::open(&alone)
                .unwrap()
                .setting::<bool>("eski")
                .unwrap(),
            Some(true)
        );
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
