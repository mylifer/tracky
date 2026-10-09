use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::time::{Duration, Instant};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::dictionary::CFDictionaryGetValue;
use core_foundation_sys::number::{
    CFNumberGetValue, CFNumberRef, kCFNumberFloat64Type, kCFNumberSInt64Type,
};
use objc2_app_kit::NSRunningApplication;
use tracky_core::{ActiveWindow, ActivityProvider};

use crate::address::{AddressCache, looks_like_address};
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

/// Yanıt vermeyen bir uygulamaya yapılan AX çağrısı en çok bu kadar bekler (saniye).
/// Sistem varsayılanı 6 sn: donmuş bir uygulama öndeyken saniyelik takip döngüsü ve
/// menü çubuğu her öznitelik için bu kadar takılırdı.
const AX_TIMEOUT_SECS: f32 = 1.0;

/// Tarayıcı adresini ararken erişilebilirlik ağacında en çok bu kadar öğeye bakılır ve
/// en çok bu kadar beklenir; adres çubuğu ve web alanı ağacın üst katlarındadır.
const ADDRESS_MAX_NODES: usize = 300;
const ADDRESS_MAX_DEPTH: u32 = 14;
const ADDRESS_BUDGET: Duration = Duration::from_millis(150);

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
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> AXError;
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

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMCopyAssertionsByProcess(assertions_by_pid: *mut CFDictionaryRef) -> i32;
}

/// Ekranın uykuya geçmesini engelleyen güç beyanları (video oynatma, görüntülü görüşme).
const DISPLAY_ASSERTIONS: &[&str] = &["PreventUserIdleDisplaySleep", "NoDisplaySleepAssertion"];
/// Ekranı izlemeden bağımsız, sürekli açık tutan araçlar (süreç adı); bunlar izleme sayılmaz.
const KEEP_AWAKE_TOOLS: &[&str] = &[
    "caffeinate",
    "Amphetamine",
    "KeepingYouAwake",
    "Lungo",
    "Theine",
    "Caffeine",
    "Owly",
    "Jiggler",
    "Kum",
];

/// `kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements`
const ON_SCREEN_WINDOWS: u32 = (1 << 0) | (1 << 4);

#[derive(Debug, Default)]
pub struct SystemProvider {
    addresses: AddressCache,
}

impl ActivityProvider for SystemProvider {
    type Error = PlatformError;

    fn active_window(&mut self) -> Result<Option<ActiveWindow>, PlatformError> {
        // Arka plan iş parçacığında otomatik serbest bırakma havuzu yok; her
        // gözlemde açılmazsa Objective-C nesneleri zamanla birikir.
        self.active_window_with(&|_| true)
    }

    fn active_window_with(
        &mut self,
        read_title: &dyn Fn(&str) -> bool,
    ) -> Result<Option<ActiveWindow>, PlatformError> {
        objc2::rc::autoreleasepool(|_| active_window(read_title, &mut self.addresses))
    }

    fn idle_seconds(&mut self) -> Result<u64, PlatformError> {
        // SAFETY: Saf sorgu; izin gerektirmez.
        let secs = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
        };
        Ok(secs.max(0.0) as u64)
    }

    fn call_apps(&mut self) -> tracky_core::platform::CallApps {
        call_signals()
    }

    fn display_kept_awake(&mut self) -> bool {
        display_wake_holders().iter().any(|name| {
            !KEEP_AWAKE_TOOLS
                .iter()
                .any(|t| t.eq_ignore_ascii_case(name))
        })
    }
}

/// Bir güç beyanı (`pmset -g assertions`).
struct PowerAssertion {
    kind: String,
    /// Sahibi sürecin adı.
    process: String,
    /// Sahibi süreç.
    pid: Option<i32>,
    /// Beyanı başka süreç adına veren (coreaudiod mikrofonu kullanan uygulama adına verir).
    on_behalf_of: Option<i32>,
    /// Kullanılan kaynaklar ("audio-in" = mikrofon).
    resources: Vec<String>,
}

/// Etkin güç beyanları.
fn power_assertions() -> Vec<PowerAssertion> {
    let mut raw: CFDictionaryRef = ptr::null();
    // SAFETY: Copy kuralı; başarıda sözlük bize geçer.
    if unsafe { IOPMCopyAssertionsByProcess(&mut raw) } != 0 || raw.is_null() {
        return Vec::new();
    }
    // SAFETY: Create kuralı; sahipliği CFDictionary devralır ve bırakır.
    let by_pid: CFDictionary = unsafe { CFDictionary::wrap_under_create_rule(raw) };
    let type_key = CFString::from_static_string("AssertType");
    let name_key = CFString::from_static_string("Process Name");
    let level_key = CFString::from_static_string("AssertLevel");
    let pid_key = CFString::from_static_string("AssertPID");
    let behalf_key = CFString::from_static_string("AssertionOnBehalfOfPID");
    let resources_key = CFString::from_static_string("ResourcesUsed");
    let mut out = Vec::new();
    for list in by_pid.get_keys_and_values().1 {
        // SAFETY: Sözlük yaşadıkça değerleri geçerlidir; türü denetlenir.
        let list = unsafe { CFType::wrap_under_get_rule(list as CFTypeRef) };
        if !list.instance_of::<CFArray>() {
            continue;
        }
        for item in array_items(&list) {
            if !item.instance_of::<CFDictionary>() {
                continue;
            }
            let item = item.as_CFTypeRef() as CFDictionaryRef;
            // SAFETY: Sözlük denetlendi; anahtarlar geçerli.
            let number = |key: &CFString| unsafe { number_i64(item, key.as_concrete_TypeRef()) };
            if number(&level_key) == Some(0) {
                continue;
            }
            let pid = |key: &CFString| number(key).and_then(|p| i32::try_from(p).ok());
            out.push(PowerAssertion {
                kind: dict_string(item, &type_key).unwrap_or_default(),
                process: dict_string(item, &name_key).unwrap_or_default(),
                pid: pid(&pid_key),
                on_behalf_of: pid(&behalf_key),
                resources: dict_value(item, &resources_key)
                    .filter(|v| v.instance_of::<CFArray>())
                    .map(|v| {
                        array_items(&v)
                            .into_iter()
                            .filter_map(|r| r.downcast::<CFString>().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    out
}

/// CFArray olduğu denetlenmiş değerin öğeleri.
fn array_items(array: &CFType) -> Vec<CFType> {
    let list = array.as_CFTypeRef() as CFArrayRef;
    // SAFETY: Dizi olduğu çağıranca denetlendi; öğeler Get kuralıyla sarılır.
    (0..unsafe { CFArrayGetCount(list) })
        .map(|i| unsafe { CFType::wrap_under_get_rule(CFArrayGetValueAtIndex(list, i)) })
        .collect()
}

/// Ekranı uyanık tutan güç beyanlarının sahibi süreçlerin adları.
fn display_wake_holders() -> Vec<String> {
    power_assertions()
        .into_iter()
        .filter(|a| DISPLAY_ASSERTIONS.contains(&a.kind.as_str()))
        .map(|a| a.process)
        .collect()
}

/// Mikrofonu kullanan ve ekranı uyanık tutan uygulamalar (kimlikleri).
fn call_signals() -> tracky_core::platform::CallApps {
    let mut out = tracky_core::platform::CallApps::default();
    for a in power_assertions() {
        let list = if a.resources.iter().any(|r| r == "audio-in") {
            &mut out.microphone
        } else if DISPLAY_ASSERTIONS.contains(&a.kind.as_str()) {
            &mut out.display
        } else {
            continue;
        };
        if let Some(id) = a.on_behalf_of.or(a.pid).and_then(app_id_of_pid)
            && !list.contains(&id)
        {
            list.push(id);
        }
    }
    out
}

unsafe extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
}

/// Sürecin ait olduğu uygulamanın kimliği: yardımcı süreç (Chrome Helper, Teams WebView
/// Helper) en dıştaki `.app` paketinin kimliğini alır; oturumlardaki uygulama kimliğiyle aynı.
/// Paket dışındaki süreçte çalıştırılabilir dosyanın yolu.
fn app_id_of_pid(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: Tampon boyutuyla verildi; dönen değer yazılan bayt sayısıdır.
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    let Some(i) = path.find(".app/") else {
        return Some(path);
    };
    let bundle = &path[..i + 4];
    let id = CFURL::from_path(bundle, true)
        .and_then(core_foundation::bundle::CFBundle::new)
        .and_then(|b| {
            b.info_dictionary()
                .find(CFString::from_static_string("CFBundleIdentifier"))
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
        });
    Some(id.unwrap_or(path))
}

/// Sözlükteki değer (Get kuralıyla sarılmış).
fn dict_value(dict: CFDictionaryRef, key: &CFString) -> Option<CFType> {
    // SAFETY: `dict` geçerli bir sözlük; değer sözlük yaşadıkça geçerli (Get kuralı).
    let value = unsafe { CFDictionaryGetValue(dict, key.as_concrete_TypeRef().cast()) };
    // SAFETY: Boş değil.
    (!value.is_null()).then(|| unsafe { CFType::wrap_under_get_rule(value) })
}

/// Sözlükteki metin değeri (türü metin değilse `None`).
fn dict_string(dict: CFDictionaryRef, key: &CFString) -> Option<String> {
    dict_value(dict, key)?
        .downcast::<CFString>()
        .map(|s| s.to_string())
}

/// Sistem geneli öğeye verilen zaman aşımı tüm AX çağrıları için geçerlidir; bir kez yeter.
fn set_ax_timeout() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: Create kuralı; sistem geneli öğe geçerli.
        let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
        // SAFETY: Geçerli öğe; başarısızlık önemsiz (varsayılan süre kalır).
        unsafe { AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), AX_TIMEOUT_SECS) };
    });
}

fn active_window(
    read_title: &dyn Fn(&str) -> bool,
    addresses: &mut AddressCache,
) -> Result<Option<ActiveWindow>, PlatformError> {
    if !is_trusted() {
        return Err(PlatformError::PermissionDenied);
    }
    set_ax_timeout();
    let Some(pid) = focused_pid()? else {
        return Ok(None);
    };
    // SAFETY: Create kuralı; sahipliği CFType devralır ve bırakır.
    let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
    let Some(running) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
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

    // Başlığı okunmayan uygulamanın (gizlilik) adresi de okunmaz.
    let readable = read_title(&app_id);
    let title = if readable {
        window_title(&app)?
    } else {
        String::new()
    };
    let url = if readable && tracky_core::browser::is_browser(&app_id) {
        addresses.get(&app_id, &title, || browser_address(&app))
    } else {
        None
    };

    Ok(Some(ActiveWindow {
        app_id,
        app_name,
        title,
        url,
    }))
}

/// Tarayıcının öndeki penceresindeki adres. Safari'de web alanının `AXURL` özniteliği (tam
/// adres), Chromium tabanlılarda ve Firefox'ta adres çubuğunun metni (çoğu zaman şemasız)
/// okunur. Ağaç genişlik öncelikli, sınırlı gezilir; web içeriğinin içine inilmez.
fn browser_address(app: &CFType) -> Option<String> {
    let window = ["AXFocusedWindow", "AXMainWindow"]
        .into_iter()
        .find_map(|name| attribute(app, name).ok().flatten())?;
    let started = Instant::now();
    let mut queue = VecDeque::from([(window, 0u32)]);
    let mut visited = 0;
    let mut address_bar: Option<String> = None;
    while let Some((element, depth)) = queue.pop_front() {
        visited += 1;
        if visited > ADDRESS_MAX_NODES || started.elapsed() > ADDRESS_BUDGET {
            break;
        }
        match string_attribute(&element, "AXRole").as_deref() {
            Some("AXWebArea") => {
                let url = attribute(&element, "AXURL")
                    .ok()
                    .flatten()
                    .and_then(|u| u.downcast::<CFURL>())
                    .map(|u| u.get_string().to_string());
                if url.is_some() {
                    return url;
                }
                continue;
            }
            Some("AXTextField" | "AXComboBox") => {
                if address_bar.is_none() {
                    address_bar =
                        string_attribute(&element, "AXValue").filter(|v| looks_like_address(v));
                }
                continue;
            }
            _ => {}
        }
        if depth < ADDRESS_MAX_DEPTH {
            queue.extend(children(&element).into_iter().map(|c| (c, depth + 1)));
        }
    }
    address_bar
}

fn string_attribute(element: &CFType, name: &'static str) -> Option<String> {
    attribute(element, name)
        .ok()
        .flatten()
        .and_then(|v| v.downcast::<CFString>())
        .map(|v| v.to_string())
}

/// `AXChildren` dizisinin öğeleri.
fn children(element: &CFType) -> Vec<CFType> {
    let Ok(Some(list)) = attribute(element, "AXChildren") else {
        return Vec::new();
    };
    let array: CFArrayRef = list.as_CFTypeRef().cast();
    // SAFETY: AXChildren bir CFArray döndürür; öğeler dizinin ömrü boyunca geçerli,
    // Get kuralıyla sarılınca kendi referanslarını tutar.
    unsafe {
        (0..CFArrayGetCount(array))
            .map(|i| CFArrayGetValueAtIndex(array, i))
            .filter(|p| !p.is_null())
            .map(|p| CFType::wrap_under_get_rule(p))
            .collect()
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
    out += &format!(" ekran-uyanık={:?}", display_wake_holders());
    out += &format!(" görüşme-sinyali={:?}", call_signals());

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
    if let Some(r) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        && let Some(id) = r.bundleIdentifier().map(|s| s.to_string())
        && tracky_core::browser::is_browser(&id)
    {
        let started = Instant::now();
        let address = browser_address(&app);
        out += &format!(" adres={address:?} ({} ms)", started.elapsed().as_millis());
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

/// Uygulama simgesi: bundle id'den (ya da yürütülebilir dosya yolundan) `.app` paketini
/// bulur, Finder'ın gösterdiği simgeyi `px` noktalık PNG'ye çevirir.
pub fn app_icon(app_id: &str, px: u32) -> Option<Vec<u8>> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let workspace = NSWorkspace::sharedWorkspace();
    // Kimlik eşitlemeyle başka cihazdan da gelebilir: otomatik bağlanan ağ yollarına
    // (`/net/sunucu/...`) dokunmak bile ağa bağlanır.
    if ["/net/", "/Network/"].iter().any(|p| app_id.starts_with(p)) {
        return None;
    }
    let path = if app_id.starts_with('/') {
        // Paketsiz kimlik yürütülebilir dosyanın yoludur: simge içinde bulunduğu paketin.
        match app_id.find(".app/") {
            Some(i) => app_id[..i + 4].to_string(),
            None => app_id.to_string(),
        }
    } else {
        workspace
            .URLForApplicationWithBundleIdentifier(&NSString::from_str(app_id))?
            .path()?
            .to_string()
    };
    // Olmayan yol için AppKit genel bir belge simgesi döndürür; o simge yanıltıcı olur.
    if !std::path::Path::new(&path).exists() {
        return None;
    }
    png_of(&workspace.iconForFile(&NSString::from_str(&path)), px)
}

/// Bu Mac'in "Bu Mac Hakkında"daki görseli (modeline ve kasa rengine göre) PNG olarak.
pub fn computer_icon(px: u32) -> Option<Vec<u8>> {
    use objc2_app_kit::{NSImage, NSImageNameComputer};
    // SAFETY: AppKit'in sabit görsel adı; süreç boyunca geçerli.
    let image = NSImage::imageNamed(unsafe { NSImageNameComputer })?;
    png_of(&image, px)
}

/// Görseli `px` kenarlı kare olarak PNG'ye çevirir.
fn png_of(image: &objc2_app_kit::NSImage, px: u32) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

    let side = f64::from(px);
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side));
    // SAFETY: `rect` çağrı boyunca yaşar; bağlam ve ipucu verilmez.
    let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    // SAFETY: Boş özellik sözlüğü PNG için geçerlidir.
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}
