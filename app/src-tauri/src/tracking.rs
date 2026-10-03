//! Arka plan takip iş parçacığı ve arayüzle paylaşılan durum.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tracky_core::{Coach, EngineConfig, Goals, Nudge, PrivacySettings, Store, Tracker, UsageTotal};

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
    /// Süreli duraklatmanın bitişi.
    pub paused_until: Option<DateTime<Utc>>,
    /// Süren odak zamanlayıcısı.
    pub focus: Option<FocusState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusState {
    pub started_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub minutes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    pub app_name: String,
    pub title: String,
    /// Bu uygulamada bugün geçen toplam süre.
    pub app_seconds_today: i64,
}

/// Hedef ve hatırlatıcı ayarlarının anahtarı.
pub const GOALS_KEY: &str = "goals";
/// Süreli duraklatmanın bitişi (yoksa süresiz).
pub const PAUSE_UNTIL_KEY: &str = "pause_until";

pub enum Command {
    SetPrivacy(PrivacySettings),
    SetGoals(Goals),
    /// Durumu hemen yeniden hesapla (örn. odak zamanlayıcısı değişti).
    Refresh,
    /// Süreli duraklatma: bu anda takip kendiliğinden sürer.
    PauseUntil(Option<DateTime<Utc>>),
    Shutdown,
}

pub struct Shared {
    pub store: Mutex<Store>,
    pub status: Mutex<Status>,
}

/// Durum (menü çubuğu, toplamlar) bu kadar gözlemde bir yeniden hesaplanır.
const REFRESH_EVERY: u32 = 5;
/// Kategori limitleri bu kadar gözlemde bir denetlenir (rapor sınıflandırması gerekir).
const LIMITS_EVERY: u32 = 30;

/// `rx` kapanana ya da `Shutdown` gelene kadar saniyede bir gözlem yapar.
pub fn run(
    app: AppHandle,
    privacy: PrivacySettings,
    mut goals: Goals,
    mut pause_until: Option<DateTime<Utc>>,
    rx: Receiver<Command>,
) {
    let mut tracker = Tracker::new(
        tracky_platform::provider(),
        EngineConfig::default(),
        privacy,
    );
    let mut coach = Coach::new();
    let mut first_refresh = true;
    let mut limits_primed = false;
    let mut ticks = 0u32;
    loop {
        let mut force = false;
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Command::SetPrivacy(p)) => tracker.set_privacy(p),
            Ok(Command::SetGoals(g)) => goals = g,
            Ok(Command::Refresh) => force = true,
            Ok(Command::PauseUntil(until)) => {
                pause_until = until;
                force = true;
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        // Süreli duraklatma doldu: takibe kendiliğinden devam et.
        if pause_until.is_some_and(|t| Utc::now() >= t) {
            pause_until = None;
            let shared = app.state::<Shared>();
            let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(mut privacy) = store.privacy_settings() {
                privacy.paused = false;
                let _ = store.save_privacy_settings(&privacy);
                let _ = store.save_setting(PAUSE_UNTIL_KEY, &None::<DateTime<Utc>>);
                tracker.set_privacy(privacy);
            }
            force = true;
        }

        // Platform çağrıları kilit dışında: yanıt vermeyen bir uygulama arayüzü kilitlemesin.
        let observation = tracker.observe();
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now();
        let outcome = tracker.record(&store, now, observation);
        ticks = ticks.wrapping_add(1);
        let focus_done = store.complete_due_focus(now).ok().flatten();
        if let Some(timer) = &focus_done {
            notify_focus_done(&app, timer.planned_minutes());
            force = true;
        }
        if !force
            && !outcome.changed
            && outcome.error.is_none()
            && !ticks.is_multiple_of(REFRESH_EVERY)
        {
            continue;
        }
        let focus = store.active_focus().ok().flatten().map(|t| FocusState {
            started_at: t.start,
            ends_at: t.planned_end,
            minutes: t.planned_minutes(),
        });

        let totals = store.app_totals(start_of_today(), now).unwrap_or_default();
        let check_limits =
            !goals.limits.is_empty() && (!limits_primed || ticks.is_multiple_of(LIMITS_EVERY));
        let category_totals = check_limits.then(|| {
            store
                .category_totals(start_of_today(), now)
                .unwrap_or_default()
        });
        let limit_names: std::collections::HashMap<String, String> = if check_limits {
            store
                .tags()
                .unwrap_or_default()
                .into_iter()
                .map(|t| (t.id, t.name))
                .collect()
        } else {
            Default::default()
        };
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
            paused_until: pause_until.filter(|_| tracker.privacy().paused),
            focus,
        };
        // Menü güncellemesi ana iş parçacığında çalışıp sonucunu bekler; burada
        // beklersek kapanışta (ana iş parçacığı bizi beklerken) kilitlenirdik.
        let today = Local::now().date_naive();
        if std::mem::take(&mut first_refresh) && status.today_seconds >= goals.daily_seconds() {
            // Gün içinde yeniden açıldı: hedef bildirimi zaten gösterilmiş olabilir.
            coach.mark_goal_notified(today);
        }
        let active = status.current.is_some() && !status.paused;
        for nudge in coach.observe(&goals, now, active, today, status.today_seconds) {
            notify(&app, &nudge, &limit_names);
        }
        if let Some(used) = category_totals {
            if std::mem::replace(&mut limits_primed, true) {
                for nudge in coach.observe_limits(&goals, today, &used) {
                    notify(&app, &nudge, &limit_names);
                }
            } else {
                // Gün içinde yeniden açıldı: geçilmiş eşikleri tekrar bildirme.
                coach.prime_limits(&goals, today, &used);
            }
        }

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

fn notify(app: &AppHandle, nudge: &Nudge, names: &std::collections::HashMap<String, String>) {
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| "Kategori".into());
    let (title, body) = match nudge {
        Nudge::TakeBreak { minutes } => (
            "Mola zamanı".to_string(),
            format!(
                "{} kesintisiz çalışıyorsun. Birkaç dakika ara ver, gözlerini dinlendir.",
                format_duration(minutes * 60)
            ),
        ),
        Nudge::GoalReached { seconds } => (
            "Günlük hedefe ulaştın".to_string(),
            format!("Bugün {} çalıştın. Tebrikler!", format_duration(*seconds)),
        ),
        Nudge::LimitNear {
            category_id,
            limit,
            used,
        } => (
            format!("{} limitine yaklaştın", name(category_id)),
            format!(
                "Bugün {} / {} kullandın. {} kaldı.",
                format_duration(*used),
                format_duration(*limit),
                format_duration(limit - used)
            ),
        ),
        Nudge::LimitReached { category_id, limit } => (
            format!("{} limiti doldu", name(category_id)),
            format!("Bugünkü {} sınırına ulaştın.", format_duration(*limit)),
        ),
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("bildirim gösterilemedi: {e}");
    }
}

fn notify_focus_done(app: &AppHandle, minutes: i64) {
    let result = app
        .notification()
        .builder()
        .title("Odak süresi bitti")
        .body(format!(
            "{} odaklandın. Kısa bir mola iyi gelir.",
            format_duration(minutes * 60)
        ))
        .show();
    if let Err(e) = result {
        eprintln!("bildirim gösterilemedi: {e}");
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
    local_midnight(Local::now().date_naive())
}

/// Günün yerel başlangıcı. Gece yarısı yaz saati geçişine denk gelip hiç
/// yaşanmıyorsa (örn. Şili, Paraguay) o günün ilk geçerli saati.
pub fn local_midnight(date: chrono::NaiveDate) -> DateTime<Utc> {
    (0..4)
        .filter_map(|h| {
            date.and_hms_opt(h, 0, 0)?
                .and_local_timezone(Local)
                .earliest()
        })
        .next()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc())
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
