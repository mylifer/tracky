//! Arka plan takip iş parçacığı ve arayüzle paylaşılan durum.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, Timelike, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tracky_core::{
    Coach, EngineConfig, Goals, Nudge, PrivacySettings, Report, Store, Tracker, UsageTotal,
};

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
/// Haftalık özetin en son gösterildiği haftanın pazartesisi.
const WEEKLY_SENT_KEY: &str = "weekly_summary_week";
/// Aktarım hatırlatmasının en son yapıldığı haftanın pazartesisi.
const EXPORT_REMINDED_KEY: &str = "export_reminder_week";
/// Gün sonu özetinin en son gösterildiği gün.
const DAY_SUMMARY_SENT_KEY: &str = "day_summary_day";
/// Günlük hedef bildiriminin en son gösterildiği gün.
const GOAL_NOTIFIED_KEY: &str = "goal_notified_day";
/// Yeni haftada bu kadar çalışılınca geçen haftanın özeti gösterilir.
const WEEKLY_AFTER_SECS: i64 = 5 * 60;
/// Geçen hafta bundan az çalışıldıysa özet gösterilmez (örn. ilk kurulum).
const WEEKLY_MIN_SECS: i64 = 60 * 60;

pub enum Command {
    SetPrivacy(PrivacySettings),
    SetGoals(Goals),
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
    let (mut weekly_sent, mut export_reminded): (Option<NaiveDate>, Option<NaiveDate>) = {
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        // Gün içinde yeniden açıldı: bugün gösterilmiş bildirimler tekrarlanmasın. Açılış saatine
        // göre tahmin edilmez; yoksa 18.00'den sonra açılan günün özeti hiç gösterilmezdi.
        if let Some(day) = store
            .setting::<NaiveDate>(DAY_SUMMARY_SENT_KEY)
            .ok()
            .flatten()
        {
            coach.mark_summary_sent(day);
        }
        if let Some(day) = store.setting::<NaiveDate>(GOAL_NOTIFIED_KEY).ok().flatten() {
            coach.mark_goal_notified(day);
        }
        (
            store.setting(WEEKLY_SENT_KEY).ok().flatten(),
            store.setting(EXPORT_REMINDED_KEY).ok().flatten(),
        )
    };
    let mut limits_primed = false;
    let mut project_goals_primed = false;
    let mut ticks = 0u32;
    loop {
        let mut force = false;
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Command::SetPrivacy(p)) => tracker.set_privacy(p),
            Ok(Command::SetGoals(g)) => goals = g,
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
        if !force
            && !outcome.changed
            && outcome.error.is_none()
            && !ticks.is_multiple_of(REFRESH_EVERY)
        {
            continue;
        }
        let totals = store.app_totals(start_of_today(), now).unwrap_or_default();
        let check_limits =
            !goals.limits.is_empty() && (!limits_primed || ticks.is_multiple_of(LIMITS_EVERY));
        let category_totals = check_limits.then(|| {
            store
                .category_totals(start_of_today(), now)
                .unwrap_or_default()
        });
        let check_projects = !goals.project_goals.is_empty()
            && (!project_goals_primed || ticks.is_multiple_of(LIMITS_EVERY));
        let project_totals = check_projects.then(|| {
            let week = week_start(Local::now().date_naive());
            store
                .project_totals(local_midnight(week), now)
                .unwrap_or_default()
        });
        let limit_names: std::collections::HashMap<String, String> =
            if check_limits || check_projects {
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
        };
        // Menü güncellemesi ana iş parçacığında çalışıp sonucunu bekler; burada
        // beklersek kapanışta (ana iş parçacığı bizi beklerken) kilitlenirdik.
        let local = Local::now();
        let today = local.date_naive();
        let minute = local.hour() * 60 + local.minute();
        let active = status.current.is_some() && !status.paused;
        for nudge in coach.observe(&goals, now, active, today, status.today_seconds) {
            if matches!(nudge, Nudge::GoalReached { .. }) {
                remember_day(&app, GOAL_NOTIFIED_KEY, today);
            }
            notify(&app, &nudge, &limit_names);
        }
        if coach
            .observe_summary(&goals, today, minute, status.today_seconds)
            .is_some()
        {
            remember_day(&app, DAY_SUMMARY_SENT_KEY, today);
            notify_day_summary(&app, &goals, now);
        }
        let week = week_start(today);
        if goals.weekly_summary
            && status.today_seconds >= WEEKLY_AFTER_SECS
            && weekly_sent != Some(week)
        {
            weekly_sent = Some(week);
            notify_week_summary(&app, week);
        }
        if let Some(at) = goals.export_reminder_at
            && export_reminded != Some(week)
            && tracky_core::export_reminder_due(today, minute, at)
        {
            export_reminded = Some(week);
            // Önerileri hesaplamak (takvim dahil) saniyelik takibi bekletmesin.
            let handle = app.clone();
            let _ = std::thread::Builder::new()
                .name("kum-export-reminder".into())
                .spawn(move || notify_unexported(&handle, week, today));
        }
        if let Some(used) = project_totals {
            if std::mem::replace(&mut project_goals_primed, true) {
                for nudge in coach.observe_project_goals(&goals, week, &used) {
                    notify(&app, &nudge, &limit_names);
                }
            } else {
                // Hafta içinde yeniden açıldı: dolmuş hedefleri tekrar bildirme.
                coach.prime_project_goals(&goals, week, &used);
            }
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

/// Günde bir kez gösterilen bildirimin gününü kaydeder (yeniden açılışta tekrarlanmasın).
fn remember_day(app: &AppHandle, key: &str, day: NaiveDate) {
    let shared = app.state::<Shared>();
    let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = store.save_setting(key, &day) {
        eprintln!("bildirim günü kaydedilemedi: {e}");
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
        // İçeriği rapordan hazırlanır: notify_day_summary.
        Nudge::DaySummary => return,
        Nudge::ProjectGoalReached { project_id, target } => (
            format!("{} haftalık hedefi doldu", name(project_id)),
            format!("Bu hafta {} çalıştın. Tebrikler!", format_duration(*target)),
        ),
    };
    match nudge {
        Nudge::TakeBreak { .. } => {}
        Nudge::ProjectGoalReached { .. } => crate::navigate_on_focus(app, "week"),
        _ => crate::navigate_on_focus(app, "day"),
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("bildirim gösterilemedi: {e}");
    }
}

/// Gün sonu özeti bildirimi; içerik bugünün raporundan.
fn notify_day_summary(app: &AppHandle, goals: &Goals, now: DateTime<Utc>) {
    let report = {
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        let start = start_of_today();
        store.report(start, now, &[start], false)
    };
    crate::navigate_on_focus(app, "day");
    let result = match report {
        Ok(r) => app
            .notification()
            .builder()
            .title("Bugünün özeti")
            .body(day_summary_body(&r, goals))
            .show(),
        Err(e) => {
            eprintln!("gün özeti hazırlanamadı: {e}");
            return;
        }
    };
    if let Err(e) = result {
        eprintln!("bildirim gösterilemedi: {e}");
    }
}

/// Toplam süre, hedef yüzdesi ve en çok zaman alan kategori.
fn day_summary_body(report: &Report, goals: &Goals) -> String {
    let mut first = format!("{} çalıştın", format_duration(report.total_seconds));
    let target = goals.daily_seconds();
    if target > 0 {
        first += &format!(" · hedef %{}", report.total_seconds * 100 / target);
    }
    let second = top_category(report);
    join_lines(first, second.into_iter().collect())
}

/// "En çok: Geliştirme (4sa)"
fn top_category(report: &Report) -> Option<String> {
    let top = report.categories.iter().max_by_key(|b| b.seconds)?;
    let name = top
        .id
        .as_ref()
        .and_then(|id| report.tags.iter().find(|t| &t.id == id))
        .map_or("Kategorisiz", |t| t.name.as_str());
    Some(format!("En çok: {name} ({})", format_duration(top.seconds)))
}

/// İlk satır ve (varsa) ` · ` ile birleştirilmiş ikinci satır.
fn join_lines(first: String, second: Vec<String>) -> String {
    if second.is_empty() {
        first
    } else {
        format!("{first}\n{}", second.join(" · "))
    }
}

/// Haftanın pazartesisi.
fn week_start(day: NaiveDate) -> NaiveDate {
    day - Days::new(u64::from(day.weekday().num_days_from_monday()))
}

/// Geçen haftanın (`week`'ten önceki 7 gün) özeti; gösterildiği hafta kaydedilir.
fn notify_week_summary(app: &AppHandle, week: NaiveDate) {
    let reports = {
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        // Uygulama yeniden açılınca aynı hafta için tekrar gösterilmesin.
        let _ = store.save_setting(WEEKLY_SENT_KEY, &week);
        let range = |first: NaiveDate| {
            let starts: Vec<_> = (0..7u64)
                .map(|i| local_midnight(first + Days::new(i)))
                .collect();
            store.report(
                starts[0],
                local_midnight(first + Days::new(7)),
                &starts,
                false,
            )
        };
        range(week - Days::new(7)).and_then(|last| Ok((last, range(week - Days::new(14))?)))
    };
    let (last, before) = match reports {
        Ok(r) => r,
        Err(e) => {
            eprintln!("haftalık özet hazırlanamadı: {e}");
            return;
        }
    };
    if last.total_seconds < WEEKLY_MIN_SECS {
        return;
    }
    crate::navigate_on_focus(app, "last-week");
    let result = app
        .notification()
        .builder()
        .title("Geçen haftanın özeti")
        .body(week_summary_body(&last, before.total_seconds))
        .show();
    if let Err(e) = result {
        eprintln!("bildirim gösterilemedi: {e}");
    }
}

/// Bu hafta zaman çizelgesine aktarılmamış günleri hatırlatır (hiç yoksa sessiz kalır).
fn notify_unexported(app: &AppHandle, week: NaiveDate, today: NaiveDate) {
    let days = crate::timesheet::unexported_days(app, week, today);
    {
        let shared = app.state::<Shared>();
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        // Uygulama yeniden açılınca aynı hafta için tekrar hatırlatılmasın.
        let _ = store.save_setting(EXPORT_REMINDED_KEY, &week);
    }
    let Some(body) = unexported_body(&days) else {
        return;
    };
    crate::navigate_on_focus(app, "timesheet");
    let result = app
        .notification()
        .builder()
        .title("Zaman çizelgesi")
        .body(body)
        .show();
    if let Err(e) = result {
        eprintln!("bildirim gösterilemedi: {e}");
    }
}

fn unexported_body(days: &[NaiveDate]) -> Option<String> {
    let names: Vec<&str> = days
        .iter()
        .map(|d| WEEKDAYS[d.weekday().num_days_from_monday() as usize])
        .collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!(
            "{one} gününün kayıtları henüz aktarılmadı. Zaman çizelgesinden gözden geçirip aktarabilirsin."
        )),
        _ => Some(format!(
            "Bu hafta {} gün henüz aktarılmadı: {}. Zaman çizelgesinden gözden geçirip aktarabilirsin.",
            names.len(),
            names.join(", ")
        )),
    }
}

const WEEKDAYS: [&str; 7] = [
    "Pazartesi",
    "Salı",
    "Çarşamba",
    "Perşembe",
    "Cuma",
    "Cumartesi",
    "Pazar",
];

/// Toplam, önceki haftaya göre değişim, en çok kategori ve en yoğun gün.
fn week_summary_body(report: &Report, previous_seconds: i64) -> String {
    let mut first = format!("{} çalıştın", format_duration(report.total_seconds));
    if previous_seconds > 0 {
        let pct = (report.total_seconds - previous_seconds) * 100 / previous_seconds;
        let sign = if pct >= 0 { "+" } else { "-" };
        first += &format!(" · önceki haftaya göre {sign}%{}", pct.abs());
    }
    let mut second: Vec<String> = top_category(report).into_iter().collect();
    if let Some(busiest) = report
        .days
        .iter()
        .filter(|d| d.seconds > 0)
        .max_by_key(|d| d.seconds)
    {
        let day = busiest
            .start
            .with_timezone(&Local)
            .weekday()
            .num_days_from_monday();
        second.push(format!("En yoğun gün: {}", WEEKDAYS[day as usize]));
    }
    join_lines(first, second)
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
    use super::{
        day_summary_body, format_duration, unexported_body, week_start, week_summary_body,
    };
    use chrono::{Local, NaiveDate, TimeZone, Utc};
    use tracky_core::report::Bucket;
    use tracky_core::report::DayBucket;
    use tracky_core::{Goals, Report, Tag, TagKind};

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(30), "<1dk");
        assert_eq!(format_duration(23 * 60 + 5), "23dk");
        assert_eq!(format_duration(3600), "1sa");
        assert_eq!(format_duration(3600 + 5 * 60), "1sa 5dk");
    }

    #[test]
    fn day_summary_names_top_category() {
        let mut report = Report {
            total_seconds: 6 * 3600,
            categories: vec![
                Bucket {
                    id: None,
                    seconds: 600,
                },
                Bucket {
                    id: Some("dev".into()),
                    seconds: 4 * 3600,
                },
            ],
            tags: vec![Tag {
                id: "dev".into(),
                kind: TagKind::Category,
                name: "Geliştirme".into(),
                color: 1,
            }],
            ..Default::default()
        };
        assert_eq!(
            day_summary_body(&report, &Goals::default()),
            "6sa çalıştın · hedef %75\nEn çok: Geliştirme (4sa)"
        );
        let no_goal = Goals {
            daily_hours: 0.0,
            ..Goals::default()
        };
        report.categories.clear();
        assert_eq!(day_summary_body(&report, &no_goal), "6sa çalıştın");
    }

    #[test]
    fn unexported_reminder_names_the_days() {
        let d = |day| NaiveDate::from_ymd_opt(2026, 9, day).unwrap();
        assert_eq!(unexported_body(&[]), None);
        assert!(
            unexported_body(&[d(28)])
                .unwrap()
                .starts_with("Pazartesi gününün")
        );
        let body = unexported_body(&[d(28), d(30)]).unwrap();
        assert!(body.contains("2 gün"), "{body}");
        assert!(body.contains("Pazartesi, Çarşamba"), "{body}");
    }

    #[test]
    fn week_starts_on_monday() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(week_start(d(2026, 10, 3)), d(2026, 9, 28)); // cumartesi
        assert_eq!(week_start(d(2026, 9, 28)), d(2026, 9, 28)); // pazartesi
        assert_eq!(week_start(d(2026, 10, 4)), d(2026, 9, 28)); // pazar
    }

    #[test]
    fn week_summary_compares_and_names_busiest_day() {
        let day = |d: u32, secs: i64| DayBucket {
            // Yerel öğle: hangi saat diliminde çalışırsa çalışsın aynı güne düşer.
            start: Local
                .with_ymd_and_hms(2026, 9, d, 12, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
            seconds: secs,
            categories: vec![],
        };
        let mut report = Report {
            total_seconds: 30 * 3600,
            days: vec![day(21, 4 * 3600), day(22, 9 * 3600), day(23, 0)],
            ..Default::default()
        };
        assert_eq!(
            week_summary_body(&report, 25 * 3600),
            "30sa çalıştın · önceki haftaya göre +%20\nEn yoğun gün: Salı"
        );
        assert_eq!(
            week_summary_body(&report, 40 * 3600),
            "30sa çalıştın · önceki haftaya göre -%25\nEn yoğun gün: Salı"
        );
        report.days.clear();
        assert_eq!(week_summary_body(&report, 0), "30sa çalıştın");
    }
}
