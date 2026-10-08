//! Menü çubuğu (macOS) / sistem tepsisi (Windows) ikonu ve menüsü.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::tracking::{Current, Status, format_duration};

const TRAY_ID: &str = "main";
/// Menü çubuğunda yer kaplamaması (ve genişliği sık değişmemesi) için proje ya da uygulama
/// adı bu uzunlukta kesilir.
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
    pause_for: Submenu<Wry>,
    pub autostart: CheckMenuItem<Wry>,
    update: MenuItem<Wry>,
    sync: MenuItem<Wry>,
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
    let pause_for = Submenu::with_id_and_items(
        app,
        "pause_for",
        "Süreli Duraklat",
        true,
        &[
            &MenuItem::with_id(app, "pause_15", "15 dakika", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_60", "1 saat", true, None::<&str>)?,
            &MenuItem::with_id(app, "pause_tomorrow", "Yarına kadar", true, None::<&str>)?,
        ],
    )?;
    let open = MenuItem::with_id(app, "open", "Raporu Aç", true, None::<&str>)?;
    let review = MenuItem::with_id(
        app,
        "review",
        "Atanmamış Süreyi Gözden Geçir",
        true,
        None::<&str>,
    )?;
    let timesheet =
        MenuItem::with_id(app, "timesheet", "Zaman Çizelgesini Aç", true, None::<&str>)?;
    let recent = MenuItem::with_id(
        app,
        "assign_recent",
        "Son Süreyi Projeye Ata…",
        true,
        None::<&str>,
    )?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Başlangıçta Aç",
        true,
        autostart,
        None::<&str>,
    )?;
    let sync = MenuItem::with_id(app, "sync", "Şimdi Eşitle", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Kum güncel", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Kum'dan Çık", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &today,
            &current,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &pause_for,
            &open,
            &review,
            &timesheet,
            &recent,
            &PredefinedMenuItem::separator(app)?,
            &autostart_item,
            &sync,
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
        pause_for,
        autostart: autostart_item,
        update,
        sync,
    });
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "toggle_pause" => {
            let paused = crate::lock(&app.state::<crate::tracking::Shared>().status).paused;
            if let Err(e) = crate::set_paused_inner(app, !paused) {
                log_error!("duraklatma değiştirilemedi: {e}");
            }
        }
        "open" => crate::show_main_window(app),
        "review" => crate::navigate(app, "review"),
        "timesheet" => crate::navigate(app, "timesheet"),
        "assign_recent" => crate::navigate(app, "assign-recent"),
        "autostart" => {
            let enabled = items(app)
                .and_then(|i| i.autostart.is_checked().ok())
                .unwrap_or(false);
            if let Err(e) = crate::set_autostart_inner(app, enabled) {
                log_error!("otomatik başlatma değiştirilemedi: {e}");
            }
        }
        "update" => {
            // Kurulum (paketi açma, uygulamayı değiştirme) ana iş parçacığını tutmasın.
            let app = app.clone();
            let _ = std::thread::Builder::new()
                .name("kum-install".into())
                .spawn(move || {
                    if let Err(e) = crate::updater::install(&app) {
                        log_error!("güncelleme kurulamadı: {e}");
                    }
                });
        }
        "sync" => crate::sync::sync_now(app.clone()),
        "quit" => app.exit(0),
        "pause_15" | "pause_60" | "pause_tomorrow" => {
            let minutes = match event.id().as_ref() {
                "pause_15" => Some(15),
                "pause_60" => Some(60),
                _ => None,
            };
            if let Err(e) = crate::pause_inner(app, true, Some(crate::pause_end(minutes))) {
                log_error!("duraklatılamadı: {e}");
            }
        }
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
        Some(c) => current_text(c),
        None => label.clone(),
    });
    let _ = items.pause_for.set_enabled(!status.paused);
    let _ = items.pause.set_text(if status.paused {
        "Devam Et"
    } else {
        "Duraklat"
    });
}

/// Eşitleme menü öğesi: son eşitleme ya da uzun süren başarısızlık uyarısı.
pub fn set_sync(app: &AppHandle, text: &str) {
    if let Some(items) = items(app) {
        let _ = items.sync.set_text(text);
    }
}

/// Güncelleme menü öğesi: hazırsa tıklanabilir.
pub fn set_update(app: &AppHandle, status: &crate::updater::UpdateStatus) {
    let Some(items) = items(app) else { return };
    match (&status.available, status.ready) {
        (Some(v), true) => {
            let _ = items.update.set_text(format!("Güncellemeyi Yükle (v{v})"));
            let _ = items.update.set_enabled(true);
        }
        (Some(v), false) if status.checking => {
            let _ = items
                .update
                .set_text(format!("Güncelleme indiriliyor (v{v})"));
            let _ = items.update.set_enabled(false);
        }
        _ => {
            let _ = items.update.set_text("Kum güncel");
            let _ = items.update.set_enabled(false);
        }
    }
}

/// Menüdeki "şu an" satırı: uygulama ve başlık; projeye düşüyorsa proje adı önde.
fn current_text(c: &Current) -> String {
    let app = if c.title.is_empty() {
        c.app_name.clone()
    } else {
        format!("{} — {}", c.app_name, truncate(&c.title, 40))
    };
    match &c.project {
        Some(p) => format!("{} · {app}", truncate(&p.name, 30)),
        None => app,
    }
}

/// Menü çubuğu metni: projeye düşen işte proje ve bugünkü süresi, değilse uygulama.
fn label(status: &Status) -> String {
    match (&status.current, status.paused) {
        (Some(c), _) => match &c.project {
            Some(p) => format!(
                "{} · {}",
                truncate(&p.name, MAX_NAME),
                format_duration(p.seconds_today)
            ),
            None => format!(
                "{} · {}",
                truncate(&c.app_name, MAX_NAME),
                format_duration(c.app_seconds_today)
            ),
        },
        (None, true) => match status.paused_until {
            Some(until) => format!(
                "Duraklatıldı · devam {}",
                until.with_timezone(&chrono::Local).format("%H:%M")
            ),
            None => "Duraklatıldı".into(),
        },
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
    use crate::tracking::CurrentProject;

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
            project: None,
        });
        assert_eq!(label(&s), "Visual Studio Cod… · 23dk");
    }

    #[test]
    fn label_prefers_project() {
        let mut current = Current {
            app_name: "Code".into(),
            title: "main.rs".into(),
            app_seconds_today: 23 * 60,
            project: Some(CurrentProject {
                id: "p".into(),
                name: "Müşteri portalı yenileme işi".into(),
                color: 1,
                seconds_today: 5 * 3600 + 20 * 60,
            }),
        };
        let s = Status {
            current: Some(current.clone()),
            ..Status::default()
        };
        assert_eq!(label(&s), "Müşteri portalı y… · 5sa 20dk");
        assert_eq!(
            current_text(&current),
            "Müşteri portalı yenileme işi · Code — main.rs"
        );
        current.project = None;
        assert_eq!(current_text(&current), "Code — main.rs");
    }
}
