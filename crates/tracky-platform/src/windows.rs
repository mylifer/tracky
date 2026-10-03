use std::collections::HashMap;
use std::ffi::c_void;
use std::iter::once;
use std::path::Path;
use std::ptr;

use tracky_core::{ActiveWindow, ActivityProvider};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId,
};
use windows_sys::core::BOOL;

use crate::{Permissions, PlatformError};

/// UWP uygulamalarını barındıran süreç; gerçek uygulama alt penceredir.
const UWP_HOST: &str = "ApplicationFrameHost.exe";
/// Ekran kilitliyken öne gelen süreçler.
const IGNORED_EXES: &[&str] = &["LockApp.exe"];

#[derive(Debug, Default)]
pub struct SystemProvider {
    /// exe yolu -> görünen ad (sürüm bilgisini her saniye okumamak için).
    names: HashMap<String, String>,
}

impl ActivityProvider for SystemProvider {
    type Error = PlatformError;

    fn active_window(&mut self) -> Result<Option<ActiveWindow>, PlatformError> {
        self.active_window_with(&|_| true)
    }

    fn active_window_with(
        &mut self,
        read_title: &dyn Fn(&str) -> bool,
    ) -> Result<Option<ActiveWindow>, PlatformError> {
        // SAFETY: Parametresiz sorgu.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return Ok(None);
        }
        let pid = window_pid(hwnd);
        let Some(mut path) = process_path(pid) else {
            return Ok(None);
        };
        if file_name(&path).eq_ignore_ascii_case(UWP_HOST)
            && let Some(child) = uwp_child_pid(hwnd, pid).and_then(process_path)
        {
            path = child;
        }
        let exe = file_name(&path);
        if IGNORED_EXES.iter().any(|i| i.eq_ignore_ascii_case(exe)) {
            return Ok(None);
        }
        let app_name = self
            .names
            .entry(path.clone())
            .or_insert_with(|| display_name(&path))
            .clone();
        let title = if read_title(&path) {
            window_text(hwnd)
        } else {
            String::new()
        };
        Ok(Some(ActiveWindow {
            app_id: path,
            app_name,
            title,
            url: None,
        }))
    }

    fn idle_seconds(&mut self) -> Result<u64, PlatformError> {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` doğru boyutla başlatıldı.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return Ok(0);
        }
        // Her iki değer de ~49 günde bir taşar; wrapping_sub bunu doğru hesaplar.
        let elapsed_ms = unsafe { GetTickCount() }.wrapping_sub(info.dwTime);
        Ok(u64::from(elapsed_ms / 1000))
    }
}

pub fn diagnose() -> String {
    let mut provider = SystemProvider::default();
    format!(
        "pencere={:?} idle={:?}s",
        provider.active_window(),
        provider.idle_seconds()
    )
}

pub fn permissions() -> Permissions {
    Permissions {
        accessibility: true,
    }
}

pub fn request_permissions() -> Permissions {
    permissions()
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: `hwnd` geçerli; tampon `len + 1` karakter.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: `pid` yazılabilir.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: Tutamaç her yolda kapatılır; tampon boyutu `size` ile verilir.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..size as usize]))
    }
}

/// UWP çerçevesinin içindeki, farklı süreçteki ilk alt pencerenin pid'i.
fn uwp_child_pid(hwnd: HWND, host_pid: u32) -> Option<u32> {
    struct Search {
        host_pid: u32,
        found: Option<u32>,
    }

    unsafe extern "system" fn visit(child: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam`, aşağıdaki `search`'e işaret eder ve çağrı boyunca yaşar.
        let search = unsafe { &mut *(lparam as *mut Search) };
        let pid = window_pid(child);
        if pid != 0 && pid != search.host_pid {
            search.found = Some(pid);
            return 0; // aramayı durdur
        }
        1
    }

    let mut search = Search {
        host_pid,
        found: None,
    };
    // SAFETY: Geri çağırma yalnızca bu çağrı süresince çalışır.
    unsafe { EnumChildWindows(hwnd, Some(visit), &mut search as *mut Search as LPARAM) };
    search.found
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// exe'nin sürüm bilgisindeki "FileDescription" (örn. "Google Chrome"); yoksa dosya adı.
fn display_name(path: &str) -> String {
    file_description(path).unwrap_or_else(|| {
        Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    })
}

fn file_description(path: &str) -> Option<String> {
    let wide = to_wide(path);
    // SAFETY: Tüm tamponlar Win32'nin döndürdüğü boyutlarla ayrılır ve okunur.
    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let block = data.as_ptr().cast::<c_void>();

        let mut languages: Vec<(u16, u16)> = Vec::new();
        let mut value: *mut c_void = ptr::null_mut();
        let mut len = 0u32;
        let query = to_wide(r"\VarFileInfo\Translation");
        if VerQueryValueW(block, query.as_ptr(), &mut value, &mut len) != 0 && len >= 4 {
            let pairs = std::slice::from_raw_parts(value as *const u16, (len / 2) as usize);
            languages.extend(
                pairs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&[lang, cp]| (lang, cp)),
            );
        }
        // Çeviri tablosu eksik olan exe'ler için yaygın varsayılanlar.
        languages.extend([(0x0409, 0x04B0), (0x0409, 0x04E4)]);

        for (lang, codepage) in languages {
            let query = to_wide(&format!(
                r"\StringFileInfo\{lang:04x}{codepage:04x}\FileDescription"
            ));
            if VerQueryValueW(block, query.as_ptr(), &mut value, &mut len) != 0 && len > 1 {
                let chars = std::slice::from_raw_parts(value as *const u16, len as usize);
                let text = String::from_utf16_lossy(chars);
                let text = text.trim_end_matches('\0').trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
        None
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}
