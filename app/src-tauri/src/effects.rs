//! Yerel pencere malzemesi: macOS'ta kenar çubuğu saydamlığı (vibrancy),
//! Windows 11'de Mica. Desteklenmiyorsa arayüz düz arka plan çizer.

use tauri::Manager;

/// Arayüze bildirilen etkin efekt: "vibrancy", "mica" ya da "none".
pub struct WindowEffect(pub &'static str);

pub fn apply(app: &tauri::App) {
    let effect = app
        .get_webview_window("main")
        .map_or("none", |window| platform(&window));
    app.manage(WindowEffect(effect));
}

#[cfg(target_os = "macos")]
fn platform(window: &tauri::WebviewWindow) -> &'static str {
    use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};
    match apply_vibrancy(
        window,
        NSVisualEffectMaterial::Sidebar,
        Some(NSVisualEffectState::FollowsWindowActiveState),
        None,
    ) {
        Ok(()) => "vibrancy",
        Err(e) => {
            eprintln!("vibrancy uygulanamadı: {e}");
            "none"
        }
    }
}

#[cfg(windows)]
fn platform(window: &tauri::WebviewWindow) -> &'static str {
    // `None`: açık/koyu sistem temasını izler. Windows 10'da desteklenmez.
    match window_vibrancy::apply_mica(window, None) {
        Ok(()) => "mica",
        Err(_) => "none",
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform(_window: &tauri::WebviewWindow) -> &'static str {
    "none"
}
