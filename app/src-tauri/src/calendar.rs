//! Outlook takvimi: yayımlanan .ics bağlantısı arka planda okunur, toplantılar zaman
//! çizelgesi önerilerine girer. Son okunan dosya diske de yazılır; çevrim dışı açılışta
//! toplantılar yine görünür. Bağlantı yalnızca bu cihazın ayarlarında tutulur.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracky_core::calendar::Calendar;
use tracky_core::timesheet::Meeting;

use crate::lock;
use crate::tracking::Shared;

const URL_KEY: &str = "calendar_url";
/// Arka planda bu aralıkla yeniden okunur.
const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Okunamazsa bu aralıklarla yeniden denenir, sonra olağan aralığa dönülür.
const RETRY: &[Duration] = &[
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
];
/// Outlook büyük takvimi yavaş üretir: bağlantı kısa, indirme uzun bekler (60 sn yetmiyordu).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const TIMEOUT: Duration = Duration::from_secs(180);
/// Yıllarca geçmişi olan takvim birkaç MB olabilir.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

type CmdResult<T> = Result<T, String>;

pub enum CalendarCommand {
    Now,
    Shutdown,
}

pub struct CalendarState {
    tx: Mutex<Sender<CalendarCommand>>,
    calendar: Mutex<Option<Arc<Calendar>>>,
    last: Mutex<Option<LastFetch>>,
    /// Son başarılı okuma; açılışta önbellek dosyasının zamanı.
    last_ok: Mutex<Option<DateTime<Utc>>>,
    /// Takvimi değiştirenleri (yenileme, bağlantı değişikliği) sıraya koyar: yenileme
    /// bağlantıyı bunu tutarken yeniden denetler, kaldırılan takvim geri yazılmaz. Yalnızca
    /// bu dosyada ve her zaman önce alınır (depo ve takvim kilitlerinden önce).
    update: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastFetch {
    at: DateTime<Utc>,
    ok: bool,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    url: Option<String>,
    /// Takvimdeki etkinlik sayısı (tekrarlar açılmadan).
    events: usize,
    last: Option<LastFetch>,
    /// Son başarılı okuma: okunamıyorsa toplantılar bu andaki takvimden.
    last_ok: Option<DateTime<Utc>>,
    /// Zaman çizelgesine alınmaması seçilen toplantı serisi sayısı.
    ignored: usize,
}

fn cache_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("calendar.ics"))
}

fn saved_url(app: &AppHandle) -> Option<String> {
    lock(&app.state::<Shared>().store)
        .setting::<Option<String>>(URL_KEY)
        .ok()
        .flatten()
        .flatten()
}

/// `webcal://` (Outlook'un "abone ol" bağlantısı) da kabul edilir.
fn normalize(url: &str) -> CmdResult<String> {
    let url = url.trim();
    let url = match url.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    };
    if !url.starts_with("https://") {
        return Err(
            "Bağlantı https:// ya da webcal:// ile başlamalı (Outlook → Ayarlar → Takvim → \
                    Paylaşılan takvimler → Takvim yayımla → ICS)"
                .into(),
        );
    }
    Ok(url)
}

/// Okuma hatasının iletisi; zaman aşımı ne kadar beklendiğini söyler.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => format!(
            "Takvim okunamadı: Outlook {} saniyede yanıt vermedi",
            TIMEOUT.as_secs()
        ),
        e => format!("Takvim okunamadı: {e}"),
    }
}

fn fetch(url: &str) -> CmdResult<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(describe)?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_string()
        .map_err(describe)?;
    if status >= 400 {
        return Err(format!(
            "Takvim okunamadı (HTTP {status}); bağlantı kaldırılmış ya da yayım durdurulmuş olabilir"
        ));
    }
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(
            "Bağlantı bir takvim (.ics) döndürmedi; HTML değil ICS bağlantısını kopyala".into(),
        );
    }
    Ok(text)
}

/// Okunamadığında sıradaki deneme ne zaman: önce kısa aralıklarla, sonra olağan aralıkla.
fn retry_after(failures: usize) -> Duration {
    match failures {
        0 => INTERVAL,
        n => RETRY.get(n - 1).copied().unwrap_or(INTERVAL),
    }
}

/// Bağlantıdan okur, ayrıştırır, önbelleğe yazar. Hata durumunda eski takvim kalır. Bağlantı
/// yoksa ya da okunduysa `true`.
fn refresh(app: &AppHandle) -> bool {
    let Some(url) = saved_url(app) else {
        return true;
    };
    let state = app.state::<CalendarState>();
    let result = fetch(&url).map(|text| {
        let cal = Calendar::parse(&text);
        (text, cal)
    });
    // İndirme sürerken takvim kaldırıldıysa ya da bağlantı değiştiyse eski takvim geri
    // yazılmasın. Denetim ile yazım arasında bağlantı değişemesin diye sıra kilidi tutulur.
    let _update = lock(&state.update);
    if saved_url(app).as_deref() != Some(url.as_str()) {
        return true;
    }
    let failing = lock(&state.last).as_ref().is_some_and(|l| !l.ok);
    let last = match result {
        Ok((text, cal)) => {
            if let Some(path) = cache_path(app) {
                let _ = std::fs::write(path, &text);
            }
            let n = cal.len();
            *lock(&state.calendar) = Some(Arc::new(cal));
            if failing {
                log_info!("takvim yeniden okundu ({n} etkinlik)");
            }
            *lock(&state.last_ok) = Some(Utc::now());
            LastFetch {
                at: Utc::now(),
                ok: true,
                message: format!("{n} etkinlik"),
            }
        }
        Err(message) => {
            // Yalnızca ilk başarısızlık yazılır (çevrimdışıyken her denemede değil); gizli
            // yayım bağlantısı günlüğe girmez.
            if !failing {
                log_error!("{}", message.replace(&url, "<takvim bağlantısı>"));
            }
            LastFetch {
                at: Utc::now(),
                ok: false,
                message,
            }
        }
    };
    let ok = last.ok;
    *lock(&state.last) = Some(last);
    drop(_update);
    let _ = app.emit("calendar", status(app));
    ok
}

fn status(app: &AppHandle) -> CalendarStatus {
    let state = app.state::<CalendarState>();
    let events = lock(&state.calendar).as_ref().map_or(0, |c| c.len());
    let ignored = lock(&app.state::<Shared>().store)
        .meeting_assignments()
        .map(|a| a.values().filter(|p| p.is_none()).count())
        .unwrap_or(0);
    CalendarStatus {
        url: saved_url(app),
        events,
        last: lock(&state.last).clone(),
        last_ok: *lock(&state.last_ok),
        ignored,
    }
}

/// `[from, to)` ile kesişen toplantılar; takvim bağlı değilse boş.
pub fn meetings(app: &AppHandle, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Meeting> {
    let calendar = lock(&app.state::<CalendarState>().calendar).clone();
    calendar.map(|c| c.meetings(from, to)).unwrap_or_default()
}

/// Takvimdeki her seriden bir örnek (toplantı önerileri geçmişten öğrenir); takvim bağlı
/// değilse boş.
pub fn series(app: &AppHandle) -> Vec<Meeting> {
    let calendar = lock(&app.state::<CalendarState>().calendar).clone();
    calendar.map(|c| c.series()).unwrap_or_default()
}

fn run(app: AppHandle, rx: Receiver<CalendarCommand>) {
    // Önce son okunan dosya: ağ beklenmeden toplantılar görünsün.
    if saved_url(&app).is_some()
        && let Some(path) = cache_path(&app)
        && let Ok(text) = std::fs::read_to_string(&path)
    {
        let cal = Calendar::parse(&text);
        let saved_at = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from);
        let state = app.state::<CalendarState>();
        let _update = lock(&state.update);
        // Ayrıştırma sürerken takvim kaldırıldıysa geri yazılmasın.
        if saved_url(&app).is_some() {
            *lock(&state.calendar) = Some(Arc::new(cal));
            *lock(&state.last_ok) = saved_at;
        }
    }
    let mut failures = 0;
    loop {
        failures = if refresh(&app) { 0 } else { failures + 1 };
        match rx.recv_timeout(retry_after(failures)) {
            Ok(CalendarCommand::Now) | Err(RecvTimeoutError::Timeout) => {}
            Ok(CalendarCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

pub fn start(app: &tauri::App) -> std::io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.manage(CalendarState {
        tx: Mutex::new(tx),
        calendar: Mutex::new(None),
        last: Mutex::new(None),
        last_ok: Mutex::new(None),
        update: Mutex::new(()),
    });
    let handle = app.handle().clone();
    std::thread::Builder::new()
        .name("kum-calendar".into())
        .spawn(move || run(handle, rx))
        .map(drop)
}

pub fn shutdown(app: &AppHandle) {
    let _ = lock(&app.state::<CalendarState>().tx).send(CalendarCommand::Shutdown);
}

#[tauri::command]
pub async fn calendar_status(app: AppHandle) -> CalendarStatus {
    status(&app)
}

/// Bağlantıyı kaydeder (önce bir kez okuyup doğrular); `None` bağlantıyı kaldırır.
#[tauri::command]
pub async fn set_calendar_url(app: AppHandle, url: Option<String>) -> CmdResult<CalendarStatus> {
    let state = app.state::<CalendarState>();
    match url.filter(|u| !u.trim().is_empty()) {
        Some(url) => {
            let url = normalize(&url)?;
            let check = url.clone();
            let text = tauri::async_runtime::spawn_blocking(move || fetch(&check))
                .await
                .map_err(|e| e.to_string())??;
            let cal = Calendar::parse(&text);
            let _update = lock(&state.update);
            if let Some(path) = cache_path(&app) {
                let _ = std::fs::write(path, &text);
            }
            *lock(&state.last_ok) = Some(Utc::now());
            lock(&app.state::<Shared>().store)
                .save_setting(URL_KEY, &Some(url))
                .map_err(|e| e.to_string())?;
            *lock(&state.last) = Some(LastFetch {
                at: Utc::now(),
                ok: true,
                message: format!("{} etkinlik", cal.len()),
            });
            *lock(&state.calendar) = Some(Arc::new(cal));
        }
        None => {
            let _update = lock(&state.update);
            lock(&app.state::<Shared>().store)
                .save_setting(URL_KEY, &None::<String>)
                .map_err(|e| e.to_string())?;
            *lock(&state.calendar) = None;
            *lock(&state.last) = None;
            *lock(&state.last_ok) = None;
            if let Some(path) = cache_path(&app) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let status = status(&app);
    let _ = app.emit("calendar", status.clone());
    Ok(status)
}

#[tauri::command]
pub fn refresh_calendar(app: AppHandle) {
    let _ = lock(&app.state::<CalendarState>().tx).send(CalendarCommand::Now);
}

/// Yoksayılan toplantıları geri getirir.
#[tauri::command]
pub async fn restore_ignored_meetings(app: AppHandle) -> CmdResult<CalendarStatus> {
    lock(&app.state::<Shared>().store)
        .restore_ignored_meetings()
        .map_err(|e| e.to_string())?;
    Ok(status(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_links_are_normalized() {
        assert_eq!(
            normalize(" webcal://outlook.office365.com/owa/calendar/x/calendar.ics").unwrap(),
            "https://outlook.office365.com/owa/calendar/x/calendar.ics"
        );
        assert!(normalize("http://example.com/a.ics").is_err());
    }

    #[test]
    fn a_failed_read_is_retried_soon_then_at_the_usual_interval() {
        assert_eq!(retry_after(0), INTERVAL);
        assert_eq!(retry_after(1), Duration::from_secs(60));
        assert_eq!(retry_after(4), Duration::from_secs(10 * 60));
        assert_eq!(retry_after(9), INTERVAL);
    }
}
