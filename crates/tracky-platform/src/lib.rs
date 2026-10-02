//! İşletim sistemine özel aktif pencere ve boşta kalma tespiti.
//!
//! - macOS: Erişilebilirlik API'si (odaktaki uygulama ve pencere başlığı) ve
//!   CoreGraphics (son girdi). Erişilebilirlik izni gerektirir.
//! - Windows: Win32 (ön plandaki pencere, süreç yolu, son girdi). İzin gerekmez.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "macos", windows)))]
use unsupported as imp;
#[cfg(windows)]
use windows as imp;

pub use imp::SystemProvider;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("Erişilebilirlik izni verilmedi")]
    PermissionDenied,
    #[error("bu işletim sistemi desteklenmiyor")]
    Unsupported,
}

/// Takip için gereken izinlerin durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// macOS Erişilebilirlik izni (Windows'ta her zaman `true`).
    pub accessibility: bool,
}

impl Permissions {
    pub fn all_granted(self) -> bool {
        self.accessibility
    }
}

/// İzinleri kullanıcıya sormadan kontrol eder.
pub fn permissions() -> Permissions {
    imp::permissions()
}

/// Eksik izinleri ister: macOS'ta sistem iletişim kutusunu gösterir ve
/// Sistem Ayarları > Gizlilik ve Güvenlik > Erişilebilirlik'e yönlendirir.
pub fn request_permissions() -> Permissions {
    imp::request_permissions()
}

// macOS'ta birim struct, Windows'ta önbellekli; ortak kurucu `default`.
#[allow(clippy::default_constructed_unit_structs)]
pub fn provider() -> SystemProvider {
    SystemProvider::default()
}
