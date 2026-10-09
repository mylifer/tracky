//! macOS: pencere Dock'ta ayrı bir küçük resme değil, Kum'un simgesine küçülür.
//!
//! macOS küçültülen pencereyi Dock'un sağına ayrı bir öğe olarak koyar ("Pencereleri
//! uygulama simgesine küçült" ayarı tüm uygulamalar için geçerlidir). Kum'da sarı düğme ve
//! ⌘M pencereyi gizler; Dock simgesine tıklamak geri açar (`RunEvent::Reopen`). Takip arka
//! planda sürer.
//!
//! Ayarlardan seçilen uygulama simgesi de burada Dock'a uygulanır ([`set_app_icon`]).

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSImage, NSWindow, NSWindowButton};
use objc2_foundation::NSData;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const MINIMIZE_ID: &str = "kum-minimize";

static APP: OnceLock<AppHandle> = OnceLock::new();

fn hide_main_window() {
    if let Some(window) = APP.get().and_then(|a| a.get_webview_window("main")) {
        let _ = window.hide();
    }
}

define_class!(
    // Sarı düğmenin hedefi: küçültmek yerine pencereyi gizler.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "KumMinimizeTarget"]
    struct MinimizeTarget;

    impl MinimizeTarget {
        #[unsafe(method(kumMinimize:))]
        fn minimize(&self, _sender: Option<&AnyObject>) {
            hide_main_window();
        }
    }
);

/// Ana pencerenin küçültme düğmesini Kum'un gizleme işlevine bağlar.
pub fn install(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(ns_window) = app
        .get_webview_window("main")
        .and_then(|w| w.ns_window().ok())
    else {
        return;
    };
    // SAFETY: Tauri'nin verdiği işaretçi canlı bir NSWindow'dur; kurulum ana iş parçacığında.
    let ns_window: &NSWindow = unsafe { &*ns_window.cast::<NSWindow>() };
    let Some(button) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let target: Retained<MinimizeTarget> = unsafe { msg_send![MinimizeTarget::alloc(mtm), init] };
    // SAFETY: hedef ve seçici uyumlu (`kumMinimize:` tek nesne alır).
    unsafe {
        button.setTarget(Some(&target));
        button.setAction(Some(sel!(kumMinimize:)));
    }
    // Düğme hedefi zayıf tutar; hedef uygulama boyunca yaşamalı.
    std::mem::forget(target);
}

/// Dock'ta, ⌘⇥ değiştiricide ve "Hakkında" penceresinde görünen simge; `None` paketteki
/// simgeye döner. Finder'daki simge paketten gelir: paketi değiştirmek imzayı (ve ona bağlı
/// Erişilebilirlik iznini) bozacağı için ona dokunulmaz. Ana iş parçacığında çağrılmalı.
pub fn set_app_icon(png: Option<&[u8]>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let image =
        png.and_then(|bytes| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes)));
    // SAFETY: ana iş parçacığındayız; nil, Apple'ın belgelediği gibi paketteki simgeye döner.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(image.as_deref()) };
}

/// Uygulama menüsü: varsayılanın aynısı, yalnızca "Küçült" (⌘M) pencereyi gizler.
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let name = app.package_info().name.clone();
    let app_menu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "nav:settings", "Ayarlar…", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Düzen",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &MenuItem::with_id(
                app,
                "nav:history",
                "Değişiklik Geçmişi…",
                true,
                Some("CmdOrCtrl+Alt+Z"),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    // Arayüzdeki sayfalar ve komutlar: seçilince arayüze "navigate" olayı gider.
    let go = Submenu::with_items(
        app,
        "Git",
        true,
        &[
            &MenuItem::with_id(
                app,
                "nav:palette",
                "Komut Paleti…",
                true,
                Some("CmdOrCtrl+K"),
            )?,
            &MenuItem::with_id(app, "nav:search", "Ara", true, Some("CmdOrCtrl+F"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "nav:day", "Gün", true, Some("CmdOrCtrl+1"))?,
            &MenuItem::with_id(app, "nav:week", "Hafta", true, Some("CmdOrCtrl+2"))?,
            &MenuItem::with_id(app, "nav:month", "Ay", true, Some("CmdOrCtrl+3"))?,
            &MenuItem::with_id(
                app,
                "nav:timesheet",
                "Zaman Çizelgesi",
                true,
                Some("CmdOrCtrl+4"),
            )?,
            &MenuItem::with_id(app, "nav:review", "Gözden Geçir", true, Some("CmdOrCtrl+5"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "nav:today", "Bugün", true, Some("CmdOrCtrl+T"))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Pencere",
        true,
        &[
            &MenuItem::with_id(app, MINIMIZE_ID, "Küçült", true, Some("CmdOrCtrl+M"))?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &go, &window])
}

pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: &MenuEvent) {
    let id = event.id().as_ref();
    if id == MINIMIZE_ID {
        hide_main_window();
    } else if let Some(target) = id.strip_prefix("nav:") {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        let _ = app.emit("navigate", target);
    }
}
