//! macOS: pencere Dock'ta ayrı bir küçük resme değil, Kum'un simgesine küçülür.
//!
//! macOS küçültülen pencereyi Dock'un sağına ayrı bir öğe olarak koyar ("Pencereleri
//! uygulama simgesine küçült" ayarı tüm uygulamalar için geçerlidir). Kum'da sarı düğme ve
//! ⌘M pencereyi gizler; Dock simgesine tıklamak geri açar (`RunEvent::Reopen`). Takip arka
//! planda sürer.
//!
//! Ayarlardan seçilen uygulama simgesi de burada Dock'a ([`set_app_icon`]) ve Finder'a
//! ([`set_finder_icon`]) uygulanır; paket simgesi değişince sistemin simge önbelleği
//! sıfırlanır ([`refresh_icon_cache`]).

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSImage, NSWindow, NSWindowButton, NSWorkspace, NSWorkspaceIconCreationOptions,
};
use objc2_foundation::{NSData, NSString};
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

/// Dock'ta, ⌘⇥ değiştiricide ve "Hakkında" penceresinde görünen simge. Paketteki simge için
/// de görsel verilir: nil verilince Dock, sabitlenmiş Kum'un güncellemeden önceki simgesini
/// kendi önbelleğinden gösteriyor. Ana iş parçacığında çağrılmalı.
pub fn set_app_icon(png: &[u8]) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png));
    // SAFETY: ana iş parçacığındayız; görsel çözülemezse nil, paketteki simgeye döner.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(image.as_deref()) };
}

/// Finder'da, Uygulamalar klasöründe, Launchpad'de ve Dock'ta (Kum kapalıyken) görünen simge.
/// `Some` pakete özel simge koyar, `None` kaldırır (paketteki simge görünür). macOS bunu
/// paketin kökündeki `Icon\r` dosyasıyla yapar; dosya zaten istenen durumdaysa paket yazılmaz.
/// Güncelleme paketi yenisiyle değiştirince özel simge gider; açılışta yeniden konur.
/// Geliştirme sürümünde (paket dışında) bir şey yapmaz. Ana iş parçacığında çağrılmalı.
pub fn set_finder_icon(png: Option<&[u8]>) {
    let Some(bundle) = bundle_path() else {
        return;
    };
    let has_custom = bundle.join("Icon\r").exists();
    if has_custom == png.is_some() {
        return;
    }
    let image =
        png.and_then(|png| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)));
    if png.is_some() && image.is_none() {
        return;
    }
    let path = NSString::from_str(&bundle.to_string_lossy());
    let ok = NSWorkspace::sharedWorkspace().setIcon_forFile_options(
        image.as_deref(),
        &path,
        NSWorkspaceIconCreationOptions(0),
    );
    if ok {
        log_info!("Finder simgesi güncellendi: {}", bundle.display());
        // Finder ve Dock simgeyi IconServices önbelleğinden alır; o sıfırlanmadan (Finder'ı
        // yeniden başlatmak yetmiyor) eski simge görünmeye devam ediyor.
        refresh_icon_cache(&["Finder", "Dock"]);
    } else {
        // Örneğin paket başka bir kullanıcıya ait ve yazılamıyor.
        log_error!("Finder simgesi güncellenemedi: {}", bundle.display());
    }
}

/// Çalışan `.app` paketinin yolu (`…/Kum.app/Contents/MacOS/kum` → `…/Kum.app`).
fn bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(|p| p.to_path_buf())
}

/// Paket simgesi değiştikten sonra kullanıcının simge önbelleğini sıfırlar. Bildirim Merkezi
/// uygulama simgesini IconServices'ten alıyor ve kullanıcıya özgü dizin
/// (`$DARWIN_USER_CACHE_DIR/com.apple.iconservices`) onu güncellemeden önceki simgenin kaydına
/// yönlendirmeyi sürdürüyor. LaunchServices'e yeniden kayıt da, bildirim servislerini yeniden
/// başlatmak da bunu düzeltmiyor. Dizin silinir (macOS yeniden kurar) ve onu tutan servisler
/// yeniden başlatılır (launchd hemen yeniden açar; bekleyen bildirimler kaybolmaz). Arka planda
/// çalışır. `also` ile yeniden başlatılacak başka süreçler de verilebilir (Finder, Dock).
pub fn refresh_icon_cache(also: &'static [&'static str]) {
    std::thread::spawn(move || {
        let cache_dir = match std::process::Command::new("/usr/bin/getconf")
            .arg("DARWIN_USER_CACHE_DIR")
            .output()
        {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            }
            _ => {
                log_error!("simge önbelleği: kullanıcı önbellek klasörü bulunamadı");
                return;
            }
        };
        let index = std::path::Path::new(&cache_dir).join("com.apple.iconservices");
        if let Err(e) = std::fs::remove_dir_all(&index)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            // Açılışta iki sıfırlama aynı anda çalışabilir; servisler yine de yeniden başlasın.
            log_error!("simge önbelleği silinemedi: {e}");
        }
        if let Err(e) = std::process::Command::new("/usr/bin/killall")
            .args(["iconservicesagent", "NotificationCenter", "usernoted"])
            .args(also)
            .output()
        {
            log_error!("simge önbelleği: killall çalışmadı: {e}");
        }
        log_info!("simge önbelleği sıfırlandı: {}", index.display());
    });
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
