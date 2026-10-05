use tracky_core::{ActiveWindow, ActivityProvider};

use crate::{Permissions, PlatformError};

#[derive(Debug, Default)]
pub struct SystemProvider;

impl ActivityProvider for SystemProvider {
    type Error = PlatformError;

    fn active_window(&mut self) -> Result<Option<ActiveWindow>, PlatformError> {
        Err(PlatformError::Unsupported)
    }

    fn idle_seconds(&mut self) -> Result<u64, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}

pub fn diagnose() -> String {
    "bu işletim sistemi desteklenmiyor".into()
}

/// İzin kavramı yok; desteklenmeme `active_window` hatasıyla bildirilir.
pub fn permissions() -> Permissions {
    Permissions {
        accessibility: true,
    }
}

pub fn request_permissions() -> Permissions {
    permissions()
}

pub fn app_icon(_app_id: &str, _px: u32) -> Option<Vec<u8>> {
    None
}
