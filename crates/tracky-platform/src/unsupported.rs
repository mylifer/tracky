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

/// İzin kavramı yok; desteklenmeme `active_window` hatasıyla bildirilir.
pub fn permissions() -> Permissions {
    Permissions {
        accessibility: true,
    }
}

pub fn request_permissions() -> Permissions {
    permissions()
}
