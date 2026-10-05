//! Kum masaüstü uygulaması: menü çubuğunda yaşayan zaman takipçisi.

mod ai;
mod backup;
mod calendar;
mod client_report;
mod commands;
#[cfg(target_os = "macos")]
mod dock;
mod edits;
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
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
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

/// Bildirimden sonra pencere bu kadar süre içinde odaklanırsa bildirimin sayfası açılır.
const PENDING_NAV_FOR: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Bildirime tıklanınca açılacak sayfa: masaüstü bildirimleri tıklamayı uygulamaya iletmez;
/// tıklama Kum'u öne getirir, pencere odaklanınca bu sayfaya gidilir.
#[derive(Default)]
pub(crate) struct PendingNav(Mutex<Option<(&'static str, std::time::Instant)>>);

/// Bildirim gösterilirken çağrılır: pencere yakında odaklanırsa `target` açılır.
pub(crate) fn navigate_on_focus(app: &AppHandle, target: &'static str) {
    *lock(&app.state::<PendingNav>().0) = Some((target, std::time::Instant::now()));
}

/// Pencereyi gösterip arayüzde `target` sayfasını açar (gün, hafta, zaman çizelgesi…).
pub(crate) fn navigate(app: &AppHandle, target: &str) {
    show_main_window(app);
    let _ = app.emit("navigate", target);
}

/// Arka plan işlerini durdurur; takip süren oturumu yazıp bitene kadar beklenir. Olağan
/// kapanışta ve Windows'ta güncelleme kurulumundan önce (kurulum süreci kendisi sonlandırır,
/// `RunEvent::Exit` gelmez) çağrılır; ikinci çağrı bir şey yapmaz.
pub(crate) fn shutdown_workers(app: &AppHandle) {
    sync::shutdown(app);
    calendar::shutdown(app);
    let worker = app.state::<Worker>();
    let _ = worker.tx.send(Command::Shutdown);
    // Kilit join'den önce bırakılır; takip iş parçacığı bitmeyi beklerken tutulmasın.
    let handle = lock(&worker.handle).take();
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Dock'ta ve uygulama değiştiricide (⌘⇥) görünür; pencere kapansa da takip menü
    // çubuğunda sürer, Dock simgesine tıklamak pencereyi geri açar.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Regular);
    effects::apply(app);
    #[cfg(target_os = "macos")]
    dock::install(app.handle());

    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    if let Err(e) = backup::apply_pending_restore(&dir) {
        eprintln!("yedek geri yüklenemedi: {e}");
    }
    let store = Store::open(dir.join(backup::DB_FILE))?;
    // Eşitleme başlamadan: geri yüklenen veritabanının eşitleme durumunu sıfırla.
    if let Err(e) = backup::finish_restore(&dir, &store) {
        eprintln!("geri yükleme sonrası eşitleme durumu sıfırlanamadı: {e}");
    }

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
    app.manage(edits::UndoLog::default());
    app.manage(PendingNav::default());
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
    calendar::start(app)?;
    backup::start(app)?;
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
    let builder = tauri::Builder::default()
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
            set_theme,
            commands::get_report,
            commands::app_titles_between,
            commands::get_taxonomy,
            commands::save_tag,
            commands::delete_tag,
            commands::save_client,
            commands::delete_client,
            commands::set_project_client,
            commands::archive_project,
            commands::unarchive_project,
            commands::set_project_budget,
            commands::set_client_budget,
            commands::get_budgets,
            commands::add_rule,
            commands::delete_rule,
            commands::assign_app_category,
            commands::known_apps,
            commands::get_suggestions,
            commands::search,
            commands::set_range_project,
            edits::undo,
            edits::get_unassigned,
            edits::assign_unassigned,
            edits::assign_window,
            edits::ignore_unassigned,
            edits::ignored_unassigned,
            edits::preview_rule,
            edits::rule_suggestions,
            edits::dismiss_rule_suggestion,
            client_report::client_report,
            client_report::export_client_report,
            timesheet::get_timesheet_config,
            timesheet::save_timesheet_config,
            timesheet::timesheet_days,
            timesheet::pending_timesheet_days,
            timesheet::save_timesheet_entry,
            timesheet::dismiss_timesheet_entry,
            timesheet::undismiss_timesheet_entries,
            timesheet::restore_hidden_entries,
            timesheet::delete_timesheet_entry,
            timesheet::merge_timesheet_entries,
            timesheet::unmerge_timesheet_entries,
            timesheet::refresh_timesheet_entries,
            timesheet::reset_timesheet_day,
            timesheet::timesheet_details,
            timesheet::pick_timesheet_file,
            timesheet::import_timesheet_template,
            timesheet::export_timesheet,
            timesheet::undo_last_export,
            timesheet::assign_meeting,
            timesheet::calendar_meetings,
            timesheet::sheet_script,
            timesheet::connect_sheet,
            timesheet::disconnect_sheet,
            timesheet::remove_timesheet,
            ai::get_ai_settings,
            ai::save_ai_settings,
            ai::test_ai_connection,
            ai::ai_write_details,
            calendar::calendar_status,
            calendar::set_calendar_url,
            calendar::refresh_calendar,
            calendar::restore_ignored_meetings,
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
            commands::default_excluded_urls,
            commands::get_goals,
            commands::save_goals,
            commands::export_csv,
            backup::backup_status,
            backup::backup_now,
            backup::open_backup_folder,
            backup::pick_backup,
            backup::restore_backup,
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
        .setup(setup);
    // macOS: "Küçült" (⌘M) pencereyi Dock'a değil Kum'un simgesine alır (dock.rs).
    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(dock::menu)
        .on_menu_event(|app, event| dock::on_menu_event(app, &event));
    let app = builder
        .on_window_event(|window, event| {
            // Pencereyi kapatmak uygulamayı kapatmaz; takip menü çubuğunda sürer.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            if let WindowEvent::Focused(true) = event {
                let app = window.app_handle();
                let pending = lock(&app.state::<PendingNav>().0).take();
                if let Some((target, at)) = pending
                    && at.elapsed() < PENDING_NAV_FOR
                {
                    let _ = app.emit("navigate", target);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Kum başlatılamadı");

    app.run(|app, event| {
        // Pencere kapalıyken Dock simgesine tıklanınca pencereyi göster.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main_window(app);
            return;
        }
        if let RunEvent::Exit = event {
            shutdown_workers(app);
        }
    });
}
