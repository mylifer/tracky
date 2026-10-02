//! Arka plan takip iş parçacığı ve arayüzle paylaşılan durum.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracky_core::{EngineConfig, PrivacySettings, Store, Tracker, UsageTotal};

use crate::tray;

/// Arayüze ve menü çubuğuna yansıyan anlık durum.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub paused: bool,
    pub current: Option<Current>,
    pub today_seconds: i64,
    /// macOS Erişilebilirlik izni eksik ya da geri alınmış.
    pub needs_permission: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    pub app_name: String,
    pub title: String,
    /// Bu uygulamada bugün geçen toplam süre.
    pub app_seconds_today: i64,
}

pub enum Command {
    SetPrivacy(PrivacySettings),
    Shutdown,
}

pub struct Shared {
    pub store: Mutex<Store>,
    pub status: Mutex<Status>,
}

/// Durum (menü çubuğu, toplamlar) bu kadar gözlemde bir yeniden hesaplanır.
const REFRESH_EVERY: u32 = 5;

/// `rx` kapanana ya da `Shutdown` gelene kadar saniyede bir gözlem yapar.
pub fn run(app: AppHandle, privacy: PrivacySettings, rx: Receiver<Command>) {
    let mut tracker = Tracker::new(
        tracky_platform::provider(),
        EngineConfig::default(),
        privacy,
    );
    let mut ticks = 0u32;
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Command::SetPrivacy(p)) => tracker.set_privacy(p),
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        // Platform çağrıları kilit dışında: yanıt vermeyen bir uygulama arayüzü kilitlemesin.
        let observation = tracker.observe();
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now();
        let outcome = tracker.record(&store, now, observation);
        ticks = ticks.wrapping_add(1);
        if !outcome.changed && outcome.error.is_none() && !ticks.is_multiple_of(REFRESH_EVERY) {
            continue;
        }

        let totals = store.app_totals(start_of_today(), now).unwrap_or_default();
        drop(store);
        let status = Status {
            paused: tracker.privacy().paused,
            current: tracker.current().map(|s| Current {
                app_name: s.app_name.clone(),
                title: s.title.clone(),
                app_seconds_today: app_seconds(&totals, &s.app_id),
            }),
            today_seconds: totals.iter().map(|t| t.seconds).sum(),
            needs_permission: !tracky_platform::permissions().all_granted(),
            error: outcome.error,
        };
        // Menü güncellemesi ana iş parçacığında çalışıp sonucunu bekler; burada
        // beklersek kapanışta (ana iş parçacığı bizi beklerken) kilitlenirdik.
        let (handle, snapshot) = (app.clone(), status.clone());
        let _ = app.run_on_main_thread(move || tray::update(&handle, &snapshot));
        let _ = app.emit("status", &status);
        *shared.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }

    let shared = app.state::<Shared>();
    let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = tracker.shutdown(&store, Utc::now()) {
        eprintln!("{e}");
    }
}

fn app_seconds(totals: &[UsageTotal], app_id: &str) -> i64 {
    totals
        .iter()
        .find(|t| t.key == app_id)
        .map_or(0, |t| t.seconds)
}

/// Yerel saatle bugünün başlangıcı.
pub fn start_of_today() -> DateTime<Utc> {
    Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).earliest())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

/// "23dk", "1sa 5dk", "<1dk"
pub fn format_duration(secs: i64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => "<1dk".into(),
        (0, m) => format!("{m}dk"),
        (h, 0) => format!("{h}sa"),
        (h, m) => format!("{h}sa {m}dk"),
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(30), "<1dk");
        assert_eq!(format_duration(23 * 60 + 5), "23dk");
        assert_eq!(format_duration(3600), "1sa");
        assert_eq!(format_duration(3600 + 5 * 60), "1sa 5dk");
    }
}
