//! Menü çubuğu (macOS) / sistem tepsisi (Windows) ikonu ve menüsü.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::tracking::{Status, format_duration};

const TRAY_ID: &str = "main";
/// Menü çubuğunda yer kaplamaması için uygulama adı bu uzunlukta kesilir.
const MAX_NAME: usize = 18;

/// Durum değiştikçe metni güncellenen menü öğeleri.
///
/// Öğe metodları ana iş parçacığında çalışıp sonucu bekler; bu yüzden kilit
/// tutulurken çağrılmazlar (`items()` bir kopya döndürür), yoksa ana iş
/// parçacığı aynı kilidi beklerken kilitlenme olur.
#[derive(Clone)]
pub struct Items {
    today: MenuItem<Wry>,
    current: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    pub autostart: CheckMenuItem<Wry>,
    update: MenuItem<Wry>,
}

pub struct TrayItems(pub Mutex<Option<Items>>);

pub fn items(app: &AppHandle) -> Option<Items> {
    app.state::<TrayItems>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn create(app: &AppHandle, autostart: bool) -> tauri::Result<()> {
    let today = MenuItem::with_id(app, "today", "Bugün: —", false, None::<&str>)?;
    let current = MenuItem::with_id(app, "current", "Başlıyor…", false, None::<&str>)?;
    let pause = MenuItem::with_id(app, "toggle_pause", "Duraklat", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Raporu Aç", true, None::<&str>)?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Başlangıçta Aç",
        true,
        autostart,
        None::<&str>,
    )?;
    let update = MenuItem::with_id(app, "update", "Kum güncel", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Kum'dan Çık", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &today,
            &current,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &autostart_item,
            &update,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?)
        .icon_as_template(true)
        .tooltip("Kum")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;

    *app.state::<TrayItems>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(Items {
        today,
        current,
        pause,
        autostart: autostart_item,
        update,
    });
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "toggle_pause" => {
            let paused = app
                .state::<crate::tracking::Shared>()
                .status
                .lock()
                .map(|s| s.paused)
                .unwrap_or(false);
            if let Err(e) = crate::set_paused_inner(app, !paused) {
                eprintln!("duraklatma değiştirilemedi: {e}");
            }
        }
        "open" => crate::show_main_window(app),
        "autostart" => {
            let enabled = items(app)
                .and_then(|i| i.autostart.is_checked().ok())
                .unwrap_or(false);
            if let Err(e) = crate::set_autostart_inner(app, enabled) {
                eprintln!("otomatik başlatma değiştirilemedi: {e}");
            }
        }
        "update" => {
            if let Err(e) = crate::updater::install(app) {
                eprintln!("güncelleme kurulamadı: {e}");
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Menü çubuğu metnini ve menüyü duruma göre günceller.
pub fn update(app: &AppHandle, status: &Status) {
    let label = label(status);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // macOS'ta ikonun yanında yazı; Windows'ta yazı desteklenmez, ipucu kullanılır.
        let _ = tray.set_title(Some(&label));
        let _ = tray.set_tooltip(Some(format!(
            "Kum — {label}\nBugün: {}",
            format_duration(status.today_seconds)
        )));
    }
    let Some(items) = items(app) else { return };
    let _ = items
        .today
        .set_text(format!("Bugün: {}", format_duration(status.today_seconds)));
    let _ = items.current.set_text(match &status.current {
        Some(c) if c.title.is_empty() => c.app_name.clone(),
        Some(c) => format!("{} — {}", c.app_name, truncate(&c.title, 40)),
        None => label.clone(),
    });
    let _ = items.pause.set_text(if status.paused {
        "Devam Et"
    } else {
        "Duraklat"
    });
}

/// Güncelleme menü öğesi: hazırsa tıklanabilir.
pub fn set_update(app: &AppHandle, status: &crate::updater::UpdateStatus) {
    let Some(items) = items(app) else { return };
    match (&status.available, status.ready) {
        (Some(v), true) => {
            let _ = items.update.set_text(format!("Güncellemeyi Yükle (v{v})"));
            let _ = items.update.set_enabled(true);
        }
        _ => {
            let _ = items.update.set_text("Kum güncel");
            let _ = items.update.set_enabled(false);
        }
    }
}

fn label(status: &Status) -> String {
    match (&status.current, status.paused) {
        (Some(c), _) => format!(
            "{} · {}",
            truncate(&c.app_name, MAX_NAME),
            format_duration(c.app_seconds_today)
        ),
        (None, true) => "Duraklatıldı".into(),
        (None, false) if status.needs_permission => "İzin gerekli".into(),
        (None, false) => "Boşta".into(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracking::Current;

    #[test]
    fn labels() {
        let mut s = Status::default();
        assert_eq!(label(&s), "Boşta");
        s.needs_permission = true;
        assert_eq!(label(&s), "İzin gerekli");
        s.paused = true;
        assert_eq!(label(&s), "Duraklatıldı");
        s.current = Some(Current {
            app_name: "Visual Studio Code Insiders".into(),
            title: String::new(),
            app_seconds_today: 23 * 60,
        });
        assert_eq!(label(&s), "Visual Studio Cod… · 23dk");
    }
}
