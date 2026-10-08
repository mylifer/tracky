//! Uygulama günlüğü (veri klasöründe `kum.log`): takip, izin, takvim, aktarma, yedek ve
//! arayüz hataları. Dosya büyüyünce bir öncekini `.1` olarak saklar. Günlük yazılamazsa
//! hiçbir şey etkilenmez; geliştirirken satırlar ayrıca standart hataya düşer.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager};

pub const LOG_FILE: &str = "kum.log";
const MAX_BYTES: u64 = 1024 * 1024;

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Aynı anda yazan iş parçacıklarının satırları karışmasın, dönüş tek kez yapılsın.
static WRITE: Mutex<()> = Mutex::new(());

/// Günlüğün yazılacağı klasör; açılışta bir kez.
pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn path() -> Option<PathBuf> {
    DIR.get().map(|d| d.join(LOG_FILE))
}

pub fn write(level: &str, message: &str) {
    eprintln!("{level} {message}");
    let Some(path) = path() else {
        return;
    };
    let _guard = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        // Çok satırlı iletiler (yığın izi) tek kayıtta kalsın diye girintilenir.
        let message = message.trim_end().replace('\n', "\n    ");
        let _ = writeln!(file, "{now} {level} {message}");
    }
}

/// `log_error!("yedek alınamadı: {e}")`
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::applog::write("HATA", &format!($($arg)*)) };
}

/// `log_info!("takip izni geri geldi")`
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::applog::write("BİLGİ", &format!($($arg)*)) };
}

/// Dosyanın son `n` satırı (yoksa boş).
pub fn tail(path: &std::path::Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Arayüzden gelen hata (yakalanmayan istisna, çizim hatası, başarısız komut).
#[tauri::command]
pub fn log_client(level: String, message: String) {
    let level = if level == "error" {
        "ARAYÜZ"
    } else {
        "ARAYÜZ-BİLGİ"
    };
    // Bozuk bir döngü günlüğü şişirmesin.
    let message: String = message.chars().take(4000).collect();
    write(level, &message);
}

/// Hata bildirirken yapıştırılacak özet: sürüm, sistem, izinler, takip ve eşitleme
/// durumu, veritabanı boyutu ve günlüklerin sonu. Gizli bilgi (jeton, anahtar) içermez.
#[tauri::command]
pub fn diagnostics(app: AppHandle) -> String {
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(format!(
        "Kum {} · {} {} · {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z"),
    ));
    let permissions = tracky_platform::permissions();
    line(format!("İzinler: {permissions:?}"));
    let status = crate::lock(&app.state::<crate::tracking::Shared>().status).clone();
    line(format!(
        "Takip: duraklatıldı={} etkin={} hata={}",
        status.paused,
        status.current.is_some(),
        status.error.as_deref().unwrap_or("-"),
    ));
    if let Ok(dir) = app.path().app_data_dir() {
        let size = |name: &str| {
            std::fs::metadata(dir.join(name))
                .map(|m| format!("{:.1} MB", m.len() as f64 / 1e6))
                .unwrap_or_else(|_| "-".into())
        };
        line(format!(
            "Veritabanı: {} (WAL {})",
            size(crate::backup::DB_FILE),
            size(&format!("{}-wal", crate::backup::DB_FILE)),
        ));
        line(String::new());
        line("— kum.log (son 150 satır) —".into());
        line(tail(&dir.join(LOG_FILE), 150));
        line(String::new());
        line("— sync.log (son 30 satır) —".into());
        line(tail(&dir.join("sync.log"), 30));
    }
    out
}

/// Günlük dosyasını Finder / Gezgin'de gösterir.
#[tauri::command]
pub fn reveal_log() {
    if let Some(path) = path() {
        if !path.exists() {
            write("BİLGİ", "günlük açıldı");
        }
        crate::commands::reveal(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_keeps_last_lines() {
        let dir = std::env::temp_dir().join(format!("kum-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.log");
        std::fs::write(&path, "a\nb\nc\n").unwrap();
        assert_eq!(tail(&path, 2), "b\nc");
        assert_eq!(tail(&path, 10), "a\nb\nc");
        assert_eq!(tail(&dir.join("yok.log"), 3), "");
        let _ = std::fs::remove_dir_all(dir);
    }
}
