//! Kum masaüstü uygulaması: menü çubuğunda yaşayan zaman takipçisi.

mod commands;
mod effects;
mod sync;
mod timesheet;
mod tracking;
mod tray;
mod updater;

use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tracky_core::Store;

use tracking::{Command, Shared, Status};
use tray::TrayItems;

/// Otomatik başlatmada pencere açılmasın diye verilen argüman.
const HIDDEN_ARG: &str = "--hidden";
const ONBOARDED_KEY: &str = "onboarded";
/// Otomatik başlatma ilk çalıştırmada bir kez açılır; sonra kullanıcının seçimi korunur.
const AUTOSTART_INIT_KEY: &str = "autostart_initialized";
/// Görünüm tercihi: "system", "light" ya da "dark".
const THEME_KEY: &str = "theme";

/// Takip iş parçacığına erişim; kapanışta son oturumun yazılmasını bekleriz.
pub(crate) struct Worker {
    pub(crate) tx: Sender<Command>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    platform: &'static str,
    accessibility: bool,
    onboarded: bool,
    autostart: bool,
    /// Pencere malzemesi: "vibrancy", "mica" ya da "none".
    effect: &'static str,
    /// Görünüm tercihi: "system", "light" ya da "dark".
    theme: String,
    tracking: Status,
}

#[tauri::command]
async fn get_status(app: AppHandle) -> Result<AppStatus, String> {
    let shared = app.state::<Shared>();
    let (onboarded, theme) = {
        let store = lock(&shared.store);
        (
            store
                .setting::<bool>(ONBOARDED_KEY)
                .map_err(|e| e.to_string())?
                .unwrap_or(false),
            theme_setting(&store),
        )
    };
    Ok(AppStatus {
        platform: std::env::consts::OS,
        accessibility: tracky_platform::permissions().accessibility,
        onboarded,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        effect: app.state::<effects::WindowEffect>().0,
        theme,
        tracking: lock(&shared.status).clone(),
    })
}

fn theme_setting(store: &Store) -> String {
    store
        .setting::<String>(THEME_KEY)
        .ok()
        .flatten()
        .filter(|t| matches!(t.as_str(), "light" | "dark"))
        .unwrap_or_else(|| "system".into())
}

/// Pencerenin (ve macOS vibrancy / Windows Mica malzemesinin) temasını uygular.
fn apply_theme(app: &AppHandle, theme: &str) {
    let theme = match theme {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_theme(theme);
    }
}

/// Görünüm tercihini kaydeder ve pencereye uygular.
#[tauri::command]
async fn set_theme(app: AppHandle, theme: String) -> Result<(), String> {
    let theme = match theme.as_str() {
        "light" | "dark" => theme,
        _ => "system".to_string(),
    };
    lock(&app.state::<Shared>().store)
        .save_setting(THEME_KEY, &theme)
        .map_err(|e| e.to_string())?;
    apply_theme(&app, &theme);
    Ok(())
}

/// macOS'ta sistem izin penceresini gösterir.
#[tauri::command]
fn request_accessibility() -> bool {
    tracky_platform::request_permissions().accessibility
}

/// Sistem Ayarları > Gizlilik ve Güvenlik > Erişilebilirlik bölümünü açar.
#[tauri::command]
fn open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_autostart_inner(&app, enabled)
}

#[tauri::command]
async fn set_paused(app: AppHandle, paused: bool) -> Result<(), String> {
    set_paused_inner(&app, paused)
}

/// `minutes` dakika duraklatır; `None` yarına (yerel gece yarısına) kadar.
#[tauri::command]
async fn pause_for(app: AppHandle, minutes: Option<u32>) -> Result<(), String> {
    pause_inner(&app, true, Some(pause_end(minutes)))
}

pub(crate) fn pause_end(minutes: Option<u32>) -> chrono::DateTime<chrono::Utc> {
    match minutes {
        Some(m) => chrono::Utc::now() + chrono::Duration::minutes(i64::from(m.clamp(1, 24 * 60))),
        None => {
            let tomorrow = chrono::Local::now().date_naive() + chrono::Days::new(1);
            tracking::local_midnight(tomorrow)
        }
    }
}

#[tauri::command]
async fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    lock(&app.state::<Shared>().store)
        .save_setting(ONBOARDED_KEY, &true)
        .map_err(|e| e.to_string())
}

/// `minutes` dakikalık odak zamanlayıcısı başlatır.
#[tauri::command]
async fn start_focus(app: AppHandle, minutes: u32) -> Result<(), String> {
    start_focus_inner(&app, minutes)
}

#[tauri::command]
async fn stop_focus(app: AppHandle) -> Result<(), String> {
    stop_focus_inner(&app)
}

pub(crate) fn start_focus_inner(app: &AppHandle, minutes: u32) -> Result<(), String> {
    lock(&app.state::<Shared>().store)
        .start_focus(minutes, chrono::Utc::now())
        .map_err(|e| e.to_string())?;
    refresh_status(app)
}

pub(crate) fn stop_focus_inner(app: &AppHandle) -> Result<(), String> {
    lock(&app.state::<Shared>().store)
        .stop_focus(chrono::Utc::now())
        .map_err(|e| e.to_string())?;
    refresh_status(app)
}

fn refresh_status(app: &AppHandle) -> Result<(), String> {
    app.state::<Worker>()
        .tx
        .send(tracking::Command::Refresh)
        .map_err(|e| e.to_string())
}

/// Kum'un kendi izinleriyle ham gözlem; başlık okunamıyorsa nedenini gösterir.
#[tauri::command]
async fn diagnose() -> String {
    tracky_platform::diagnose()
}

pub(crate) fn set_autostart_inner(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| e.to_string())?;
    if let Some(items) = tray::items(app) {
        let _ = items.autostart.set_checked(enabled);
    }
    Ok(())
}

/// Duraklatma gizlilik ayarının parçasıdır; kalıcıdır ve uygulama yeniden açıldığında korunur.
pub(crate) fn set_paused_inner(app: &AppHandle, paused: bool) -> Result<(), String> {
    pause_inner(app, paused, None)
}

/// `until` verilirse takip o ana kadar duraklar, sonra kendiliğinden sürer.
pub(crate) fn pause_inner(
    app: &AppHandle,
    paused: bool,
    until: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), String> {
    let until = until.filter(|_| paused);
    let shared = app.state::<Shared>();
    let privacy = {
        let store = lock(&shared.store);
        let mut privacy = store.privacy_settings().map_err(|e| e.to_string())?;
        privacy.paused = paused;
        store
            .save_privacy_settings(&privacy)
            .map_err(|e| e.to_string())?;
        store
            .save_setting(tracking::PAUSE_UNTIL_KEY, &until)
            .map_err(|e| e.to_string())?;
        privacy
    };
    let tx = &app.state::<Worker>().tx;
    tx.send(Command::SetPrivacy(privacy))
        .and_then(|_| tx.send(Command::PauseUntil(until)))
        .map_err(|e| e.to_string())?;
    let status = {
        let mut status = lock(&shared.status);
        status.paused = paused;
        status.paused_until = until;
        status.clone()
    };
    tray::update(app, &status);
    Ok(())
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Dock'ta ikon yok; uygulama yalnızca menü çubuğunda yaşar.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    effects::apply(app);

    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let store = Store::open(dir.join("kum.db"))?;

    if store.setting::<bool>(AUTOSTART_INIT_KEY)?.is_none() {
        if let Err(e) = app.autolaunch().enable() {
            eprintln!("otomatik başlatma açılamadı: {e}");
        }
        store.save_setting(AUTOSTART_INIT_KEY, &true)?;
    }
    let privacy = store.privacy_settings()?;
    let goals = store
        .setting::<tracky_core::Goals>(tracking::GOALS_KEY)?
        .unwrap_or_default();
    let onboarded = store.setting::<bool>(ONBOARDED_KEY)?.unwrap_or(false);
    let pause_until = store
        .setting::<Option<chrono::DateTime<chrono::Utc>>>(tracking::PAUSE_UNTIL_KEY)?
        .flatten()
        .filter(|_| privacy.paused);
    apply_theme(app.handle(), &theme_setting(&store));

    app.manage(Shared {
        store: Mutex::new(store),
        status: Mutex::new(Status {
            paused: privacy.paused,
            ..Default::default()
        }),
    });
    app.manage(TrayItems(Mutex::new(None)));
    tray::create(app.handle(), app.autolaunch().is_enabled().unwrap_or(false))?;

    let (tx, rx) = mpsc::channel();
    let handle = app.handle().clone();
    let worker = std::thread::Builder::new()
        .name("kum-tracker".into())
        .spawn(move || tracking::run(handle, privacy, goals, pause_until, rx))?;
    app.manage(Worker {
        tx,
        handle: Mutex::new(Some(worker)),
    });
    sync::start(app)?;
    updater::start(app);

    // Karşılama tamamlanmadıysa ya da izin eksikse pencereyi göster;
    // otomatik başlatmada (--hidden) yalnızca menü çubuğunda kal.
    let launched_hidden = std::env::args().any(|a| a == HIDDEN_ARG);
    let needs_setup = !onboarded || !tracky_platform::permissions().all_granted();
    if needs_setup || !launched_hidden {
        show_main_window(app.handle());
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_status,
            request_accessibility,
            open_accessibility_settings,
            set_autostart,
            set_paused,
            pause_for,
            complete_onboarding,
            diagnose,
            start_focus,
            set_theme,
            stop_focus,
            commands::get_report,
            commands::app_titles_between,
            commands::get_taxonomy,
            commands::save_tag,
            commands::delete_tag,
            commands::add_rule,
            commands::delete_rule,
            commands::assign_app_category,
            commands::known_apps,
            commands::get_suggestions,
            commands::search,
            commands::set_range_project,
            timesheet::get_timesheet_config,
            timesheet::save_timesheet_config,
            timesheet::timesheet_days,
            timesheet::approve_timesheet_day,
            timesheet::save_timesheet_entry,
            timesheet::delete_timesheet_entry,
            timesheet::timesheet_details,
            timesheet::pick_timesheet_file,
            timesheet::import_timesheet_template,
            timesheet::export_timesheet,
            commands::get_trends,
            commands::export_search,
            commands::accept_project_suggestion,
            commands::accept_category_suggestion,
            commands::dismiss_suggestion,
            commands::set_range_category,
            commands::delete_range,
            commands::add_manual_entry,
            commands::get_privacy,
            commands::save_privacy,
            commands::get_goals,
            commands::save_goals,
            commands::export_csv,
            sync::sync_status,
            sync::sync_configure,
            sync::sync_sign_in,
            sync::sync_sign_out,
            sync::sync_disconnect,
            sync::sync_now,
            updater::update_status,
            updater::check_update,
            updater::install_update,
        ])
        .setup(setup)
        .on_window_event(|window, event| {
            // Pencereyi kapatmak uygulamayı kapatmaz; takip menü çubuğunda sürer.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Kum başlatılamadı");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            sync::shutdown(app);
            let worker = app.state::<Worker>();
            let _ = worker.tx.send(Command::Shutdown);
            // Kilit join'den önce bırakılır; takip iş parçacığı bitmeyi beklerken tutulmasın.
            let handle = lock(&worker.handle).take();
            if let Some(handle) = handle {
                let _ = handle.join();
            }
        }
    });
}
