use std::ffi::c_void;
use std::ptr;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::dictionary::CFDictionaryGetValue;
use core_foundation_sys::number::{
    CFNumberGetValue, CFNumberRef, kCFNumberFloat64Type, kCFNumberSInt64Type,
};
use objc2_app_kit::NSRunningApplication;
use tracky_core::{ActiveWindow, ActivityProvider};

use crate::{Permissions, PlatformError};

type AXUIElementRef = *const c_void;
type AXError = i32;

const AX_SUCCESS: AXError = 0;
const AX_API_DISABLED: AXError = -25211;

/// `kCGEventSourceStateCombinedSessionState`
const COMBINED_SESSION_STATE: i32 = 0;
/// `kCGEventSourceStateHIDSystemState`
const HID_SYSTEM_STATE: i32 = 1;
/// `kCGAnyInputEventType`
const ANY_INPUT_EVENT: u32 = !0;

/// Ekran kilitli / ekran koruyucu açıkken öne gelen sistem süreçleri.
const IGNORED_BUNDLES: &[&str] = &["com.apple.loginwindow", "com.apple.ScreenSaver.Engine"];

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowAlpha: CFStringRef;
}

/// `kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements`
const ON_SCREEN_WINDOWS: u32 = (1 << 0) | (1 << 4);

#[derive(Debug, Default)]
pub struct SystemProvider;

impl ActivityProvider for SystemProvider {
    type Error = PlatformError;

    fn active_window(&mut self) -> Result<Option<ActiveWindow>, PlatformError> {
        if !is_trusted() {
            return Err(PlatformError::PermissionDenied);
        }
        let Some(pid) = focused_pid()? else {
            return Ok(None);
        };
        // SAFETY: Create kuralı; sahipliği CFType devralır ve bırakır.
        let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
        let Some(running) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        else {
            return Ok(None);
        };

        let bundle_id = running.bundleIdentifier().map(|s| s.to_string());
        if bundle_id
            .as_deref()
            .is_some_and(|id| IGNORED_BUNDLES.contains(&id))
        {
            return Ok(None);
        }
        let exe_path = running
            .executableURL()
            .and_then(|u| u.path())
            .map(|s| s.to_string());
        let app_id = bundle_id
            .or(exe_path)
            .unwrap_or_else(|| format!("pid:{pid}"));
        let app_name = running
            .localizedName()
            .map(|s| s.to_string())
            .unwrap_or_else(|| app_id.clone());

        let title = window_title(&app)?;

        Ok(Some(ActiveWindow {
            app_id,
            app_name,
            title,
            url: None,
        }))
    }

    fn idle_seconds(&mut self) -> Result<u64, PlatformError> {
        // SAFETY: Saf sorgu; izin gerektirmez.
        let secs = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
        };
        Ok(secs.max(0.0) as u64)
    }
}

/// Uygulamanın öndeki penceresinin başlığı.
///
/// Odaktaki pencere her zaman okunamıyor (bazı uygulamalar -25204 döndürür);
/// sırasıyla ana pencere ve pencere listesinin ilki denenir.
fn window_title(app: &CFType) -> Result<String, PlatformError> {
    for name in ["AXFocusedWindow", "AXMainWindow"] {
        if let Some(window) = attribute(app, name)?
            && let Some(title) = title_of(&window)?
        {
            return Ok(title);
        }
    }
    if let Some(window) = first_window(app)?
        && let Some(title) = title_of(&window)?
    {
        return Ok(title);
    }
    Ok(String::new())
}

fn title_of(window: &CFType) -> Result<Option<String>, PlatformError> {
    Ok(attribute(window, "AXTitle")?
        .and_then(|t| t.downcast::<CFString>())
        .map(|t| t.to_string())
        .filter(|t| !t.is_empty()))
}

/// `AXWindows` dizisinin ilk öğesi (öndeki pencere).
fn first_window(app: &CFType) -> Result<Option<CFType>, PlatformError> {
    let Some(list) = attribute(app, "AXWindows")? else {
        return Ok(None);
    };
    let array: CFArrayRef = list.as_CFTypeRef().cast();
    // SAFETY: AXWindows bir CFArray döndürür; öğe dizinin ömrü boyunca geçerli,
    // Get kuralıyla sarılınca kendi referansını tutar.
    unsafe {
        if CFArrayGetCount(array) == 0 {
            return Ok(None);
        }
        let first = CFArrayGetValueAtIndex(array, 0);
        Ok((!first.is_null()).then(|| CFType::wrap_under_get_rule(first)))
    }
}

/// Odaktaki uygulamanın pid'i.
///
/// Önce sistem geneli AX öğesine sorulur; bazı macOS kurulumlarında bu
/// sürekli `kAXErrorCannotComplete` (-25204) döndürdüğü için ekrandaki
/// en öndeki normal pencerenin sahibine düşülür (izin gerektirmez).
fn focused_pid() -> Result<Option<i32>, PlatformError> {
    // SAFETY: Create kuralı.
    let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
    if let Some(app) = attribute(&system, "AXFocusedApplication")? {
        let mut pid = 0;
        // SAFETY: `app` geçerli bir AXUIElement.
        if unsafe { AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) } == AX_SUCCESS {
            return Ok(Some(pid));
        }
    }
    Ok(frontmost_window_pid())
}

/// Ekranda öndeden arkaya sıralı pencerelerden ilk görünür normal (katman 0) pencerenin sahibi.
fn frontmost_window_pid() -> Option<i32> {
    // SAFETY: Copy kuralıyla dönen dizi CFType'a sarılıp bırakılır; öğeler
    // dizinin ömrü boyunca geçerlidir ve yalnızca okunur.
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_WINDOWS, 0);
        if list.is_null() {
            return None;
        }
        let _owner = CFType::wrap_under_create_rule(list.cast());
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i).cast();
            let layer = number_i64(info, kCGWindowLayer);
            let alpha = number_f64(info, kCGWindowAlpha).unwrap_or(1.0);
            if layer == Some(0) && alpha > 0.0 {
                return number_i64(info, kCGWindowOwnerPID).map(|p| p as i32);
            }
        }
        None
    }
}

/// SAFETY: `dict` geçerli bir CFDictionary olmalı.
unsafe fn number_i64(dict: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
    let mut out = 0i64;
    let value = unsafe { CFDictionaryGetValue(dict, key.cast()) } as CFNumberRef;
    (!value.is_null()
        && unsafe { CFNumberGetValue(value, kCFNumberSInt64Type, (&raw mut out).cast()) })
    .then_some(out)
}

/// SAFETY: `dict` geçerli bir CFDictionary olmalı.
unsafe fn number_f64(dict: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
    let mut out = 0f64;
    let value = unsafe { CFDictionaryGetValue(dict, key.cast()) } as CFNumberRef;
    (!value.is_null()
        && unsafe { CFNumberGetValue(value, kCFNumberFloat64Type, (&raw mut out).cast()) })
    .then_some(out)
}

/// Bir AX özniteliğini okur. Değer yoksa ya da uygulama yanıt vermiyorsa `None`.
fn attribute(element: &CFType, name: &'static str) -> Result<Option<CFType>, PlatformError> {
    match raw_attribute(element, name) {
        Ok(value) => Ok(value),
        Err(AX_API_DISABLED) => Err(PlatformError::PermissionDenied),
        Err(_) => Ok(None),
    }
}

/// Ham AX hata kodunu koruyan okuma (teşhis için).
fn raw_attribute(element: &CFType, name: &'static str) -> Result<Option<CFType>, AXError> {
    let attr = CFString::from_static_string(name);
    let mut value: CFTypeRef = ptr::null();
    // SAFETY: `element` geçerli bir AXUIElement; `value` Copy kuralıyla döner.
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            attr.as_concrete_TypeRef(),
            &mut value,
        )
    };
    match err {
        AX_SUCCESS if value.is_null() => Ok(None),
        // SAFETY: Copy kuralı; sahiplik bize geçti.
        AX_SUCCESS => Ok(Some(unsafe { CFType::wrap_under_create_rule(value) })),
        code => Err(code),
    }
}

/// Her adımın ham sonucunu tek satırda döndürür.
pub fn diagnose() -> String {
    let mut out = format!("izin={}", is_trusted());
    // SAFETY: Saf sorgular.
    let (combined, hid) = unsafe {
        (
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT),
            CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT_EVENT),
        )
    };
    out += &format!(" idle(oturum)={combined:.0}s idle(hid)={hid:.0}s");

    // SAFETY: Create kuralı.
    let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
    match raw_attribute(&system, "AXFocusedApplication") {
        Ok(Some(_)) => out += " ax-odak=tamam",
        Ok(None) => out += " ax-odak=yok",
        Err(code) => out += &format!(" ax-odak=HATA({code})"),
    }
    let window_pid = frontmost_window_pid();
    out += &format!(" pencere-listesi-pid={window_pid:?}");
    let pid = match focused_pid() {
        Ok(Some(pid)) => pid,
        Ok(None) => return out + " pid=yok",
        Err(e) => return out + &format!(" pid=HATA({e})"),
    };
    out += &format!(" pid={pid}");
    // SAFETY: Create kuralı.
    let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
    match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        Some(r) => {
            out += &format!(
                " bundle={:?} ad={:?}",
                r.bundleIdentifier().map(|s| s.to_string()),
                r.localizedName().map(|s| s.to_string())
            )
        }
        None => out += " NSRunningApplication=yok",
    }
    for name in ["AXFocusedWindow", "AXMainWindow"] {
        let short = name.trim_start_matches("AX");
        match raw_attribute(&app, name) {
            Ok(Some(w)) => match raw_attribute(&w, "AXTitle") {
                Ok(t) => {
                    let title = t
                        .and_then(|t| t.downcast::<CFString>())
                        .map(|t| t.to_string());
                    out += &format!(" {short}.başlık={title:?}");
                }
                Err(code) => out += &format!(" {short}.başlık=HATA({code})"),
            },
            Ok(None) => out += &format!(" {short}=yok"),
            Err(code) => out += &format!(" {short}=HATA({code})"),
        }
    }
    match first_window(&app) {
        Ok(Some(w)) => out += &format!(" Windows[0].başlık={:?}", title_of(&w)),
        Ok(None) => out += " Windows=boş",
        Err(e) => out += &format!(" Windows=HATA({e})"),
    }
    out += &format!(" → sonuç={:?}", window_title(&app));
    out
}

fn is_trusted() -> bool {
    // SAFETY: Saf sorgu.
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn permissions() -> Permissions {
    Permissions {
        accessibility: is_trusted(),
    }
}

pub fn request_permissions() -> Permissions {
    // SAFETY: Sistem sabiti; Get kuralı (sahiplik bizde değil).
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options =
        CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    // SAFETY: Geçerli bir CFDictionary.
    let accessibility =
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 };
    Permissions { accessibility }
}
