//! Her yerden kısayol: Kum'u öne getirip "Son süreyi ata" penceresini açar (başka bir
//! uygulamadayken biten işi projeye yazmak için). Açık/kapalı oluşu bu cihazın ayarıdır.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::lock;
use crate::tracking::Shared;

/// `bool`; yoksa açık. Eşitlenmez (kısayol bilgisayara özgü).
const ENABLED_KEY: &str = "recent_shortcut";

#[cfg(target_os = "macos")]
const ACCELERATOR: &str = "Control+Alt+Super+KeyK";
#[cfg(target_os = "macos")]
const LABEL: &str = "⌃⌥⌘K";
#[cfg(windows)]
const ACCELERATOR: &str = "Control+Alt+Shift+KeyK";
#[cfg(not(target_os = "macos"))]
const LABEL: &str = "Ctrl+Alt+Shift+K";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    enabled: bool,
    label: &'static str,
    /// Kayıt başarısızsa nedeni (ör. başka bir uygulama aynı kısayolu kullanıyor).
    error: Option<String>,
}

fn enabled(app: &AppHandle) -> bool {
    lock(&app.state::<Shared>().store)
        .setting::<bool>(ENABLED_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
}

#[cfg(any(target_os = "macos", windows))]
mod platform {
    use tauri::{AppHandle, Runtime};
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    open(app);
                }
            })
            .build()
    }

    fn open<R: Runtime>(app: &AppHandle<R>) {
        use tauri::{Emitter, Manager};
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let _ = app.emit("navigate", "assign-recent");
    }

    pub fn set(app: &AppHandle, on: bool) -> Result<(), String> {
        let shortcuts = app.global_shortcut();
        let registered = shortcuts.is_registered(super::ACCELERATOR);
        match (on, registered) {
            (true, false) => shortcuts
                .register(super::ACCELERATOR)
                .map_err(|e| e.to_string()),
            (false, true) => shortcuts
                .unregister(super::ACCELERATOR)
                .map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use tauri::AppHandle;

    pub fn set(_: &AppHandle, _: bool) -> Result<(), String> {
        Err("bu sistemde desteklenmiyor".into())
    }
}

#[cfg(any(target_os = "macos", windows))]
pub use platform::plugin;

/// Son kayıt hatası (ayarlarda gösterilir).
static ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn apply(app: &AppHandle, on: bool) {
    let result = platform::set(app, on);
    if let Err(e) = &result {
        log_error!("kısayol {LABEL} kaydedilemedi: {e}");
    }
    *lock(&ERROR) = result.err();
}

/// Açılışta: ayar açıksa kısayolu kaydeder.
pub fn init(app: &AppHandle) {
    if enabled(app) {
        apply(app, true);
    }
}

fn status(app: &AppHandle) -> ShortcutStatus {
    ShortcutStatus {
        enabled: enabled(app),
        label: LABEL,
        error: lock(&ERROR).clone(),
    }
}

#[tauri::command]
pub fn shortcut_status(app: AppHandle) -> ShortcutStatus {
    status(&app)
}

#[tauri::command]
pub fn set_shortcut_enabled(app: AppHandle, enabled: bool) -> Result<ShortcutStatus, String> {
    lock(&app.state::<Shared>().store)
        .save_setting(ENABLED_KEY, &enabled)
        .map_err(|e| e.to_string())?;
    apply(&app, enabled);
    Ok(status(&app))
}
