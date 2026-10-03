//! Kum masaüstü uygulaması: menü çubuğunda yaşayan zaman takipçisi.

mod commands;
mod sync;
mod tracking;
mod tray;
mod updater;

use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use serde::Serialize;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tracky_core::{Store, UsageTotal};

use tracking::{Command, Shared, Status};
use tray::TrayItems;

/// Otomatik başlatmada pencere açılmasın diye verilen argüman.
const HIDDEN_ARG: &str = "--hidden";
const ONBOARDED_KEY: &str = "onboarded";
/// Otomatik başlatma ilk çalıştırmada bir kez açılır; sonra kullanıcının seçimi korunur.
const AUTOSTART_INIT_KEY: &str = "autostart_initialized";

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
    tracking: Status,
}

#[tauri::command]
async fn get_status(app: AppHandle) -> Result<AppStatus, String> {
    let shared = app.state::<Shared>();
    let onboarded = lock(&shared.store)
        .setting::<bool>(ONBOARDED_KEY)
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    Ok(AppStatus {
        platform: std::env::consts::OS,
        accessibility: tracky_platform::permissions().accessibility,
        onboarded,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        tracking: lock(&shared.status).clone(),
    })
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

#[tauri::command]
async fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    lock(&app.state::<Shared>().store)
        .save_setting(ONBOARDED_KEY, &true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn today_apps(app: AppHandle) -> Result<Vec<UsageTotal>, String> {
    lock(&app.state::<Shared>().store)
        .app_totals(tracking::start_of_today(), chrono::Utc::now())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn app_titles(app: AppHandle, app_id: String) -> Result<Vec<UsageTotal>, String> {
    lock(&app.state::<Shared>().store)
        .title_totals(&app_id, tracking::start_of_today(), chrono::Utc::now())
        .map_err(|e| e.to_string())
}

/// Kum'un kendi izinleriyle ham gözlem; başlık okunamıyorsa nedenini gösterir.
#[tauri::command]
fn diagnose() -> String {
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
    let shared = app.state::<Shared>();
    let privacy = {
        let store = lock(&shared.store);
        let mut privacy = store.privacy_settings().map_err(|e| e.to_string())?;
        privacy.paused = paused;
        store
            .save_privacy_settings(&privacy)
            .map_err(|e| e.to_string())?;
        privacy
    };
    app.state::<Worker>()
        .tx
        .send(Command::SetPrivacy(privacy))
        .map_err(|e| e.to_string())?;
    let status = {
        let mut status = lock(&shared.status);
        status.paused = paused;
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
    let onboarded = store.setting::<bool>(ONBOARDED_KEY)?.unwrap_or(false);

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
        .spawn(move || tracking::run(handle, privacy, rx))?;
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
            complete_onboarding,
            today_apps,
            app_titles,
            diagnose,
            commands::get_report,
            commands::app_titles_between,
            commands::get_taxonomy,
            commands::save_tag,
            commands::delete_tag,
            commands::add_rule,
            commands::delete_rule,
            commands::assign_app_category,
            commands::known_apps,
            commands::get_privacy,
            commands::save_privacy,
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
            if let Some(handle) = lock(&worker.handle).take() {
                let _ = handle.join();
            }
        }
    });
}
