use std::ffi::c_void;
use std::ptr;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
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
}

#[derive(Debug, Default)]
pub struct SystemProvider;

impl ActivityProvider for SystemProvider {
    type Error = PlatformError;

    fn active_window(&mut self) -> Result<Option<ActiveWindow>, PlatformError> {
        if !is_trusted() {
            return Err(PlatformError::PermissionDenied);
        }
        // SAFETY: Create kuralı; sahipliği CFType devralır ve bırakır.
        let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
        let Some(app) = attribute(&system, "AXFocusedApplication")? else {
            return Ok(None);
        };

        let mut pid = 0;
        // SAFETY: `app` geçerli bir AXUIElement.
        if unsafe { AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) } != AX_SUCCESS {
            return Ok(None);
        }
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

        let title = attribute(&app, "AXFocusedWindow")?
            .map(|window| attribute(&window, "AXTitle"))
            .transpose()?
            .flatten()
            .and_then(|t| t.downcast::<CFString>())
            .map(|t| t.to_string())
            .unwrap_or_default();

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
    let app = match raw_attribute(&system, "AXFocusedApplication") {
        Ok(Some(app)) => app,
        Ok(None) => return out + " odak-uygulama=yok",
        Err(code) => return out + &format!(" odak-uygulama=HATA({code})"),
    };
    let mut pid = 0;
    // SAFETY: `app` geçerli bir AXUIElement.
    let err = unsafe { AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) };
    out += &format!(" pid={pid} (kod {err})");
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
    match raw_attribute(&app, "AXFocusedWindow") {
        Ok(Some(w)) => match raw_attribute(&w, "AXTitle") {
            Ok(t) => {
                let title = t
                    .and_then(|t| t.downcast::<CFString>())
                    .map(|t| t.to_string());
                out += &format!(" başlık={title:?}");
            }
            Err(code) => out += &format!(" başlık=HATA({code})"),
        },
        Ok(None) => out += " pencere=yok",
        Err(code) => out += &format!(" pencere=HATA({code})"),
    }
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
