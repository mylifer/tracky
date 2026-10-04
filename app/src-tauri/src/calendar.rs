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
const TIMEOUT: Duration = Duration::from_secs(60);
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

fn fetch(url: &str) -> CmdResult<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("Takvim okunamadı: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_string()
        .map_err(|e| format!("Takvim okunamadı: {e}"))?;
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

/// Bağlantıdan okur, ayrıştırır, önbelleğe yazar. Hata durumunda eski takvim kalır.
fn refresh(app: &AppHandle) {
    let Some(url) = saved_url(app) else {
        return;
    };
    let state = app.state::<CalendarState>();
    let result = fetch(&url);
    // İndirme sürerken takvim kaldırıldıysa ya da bağlantı değiştiyse eski takvim geri
    // yazılmasın.
    if saved_url(app).as_deref() != Some(url.as_str()) {
        return;
    }
    let result = result.map(|text| {
        if let Some(path) = cache_path(app) {
            let _ = std::fs::write(path, &text);
        }
        Calendar::parse(&text)
    });
    let last = match result {
        Ok(cal) => {
            let n = cal.len();
            *lock(&state.calendar) = Some(Arc::new(cal));
            LastFetch {
                at: Utc::now(),
                ok: true,
                message: format!("{n} etkinlik"),
            }
        }
        Err(message) => LastFetch {
            at: Utc::now(),
            ok: false,
            message,
        },
    };
    *lock(&state.last) = Some(last);
    let _ = app.emit("calendar", status(app));
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
        && let Some(text) = cache_path(&app).and_then(|p| std::fs::read_to_string(p).ok())
    {
        *lock(&app.state::<CalendarState>().calendar) = Some(Arc::new(Calendar::parse(&text)));
    }
    loop {
        refresh(&app);
        match rx.recv_timeout(INTERVAL) {
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
            if let Some(path) = cache_path(&app) {
                let _ = std::fs::write(path, &text);
            }
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
            lock(&app.state::<Shared>().store)
                .save_setting(URL_KEY, &None::<String>)
                .map_err(|e| e.to_string())?;
            *lock(&state.calendar) = None;
            *lock(&state.last) = None;
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
}
