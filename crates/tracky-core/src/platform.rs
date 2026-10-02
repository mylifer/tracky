use crate::model::ActiveWindow;

/// Her işletim sistemi için ayrı uygulanan gözlem arayüzü.
pub trait ActivityProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Ön plandaki pencere; ekran kilitliyse ya da pencere yoksa `None`.
    fn active_window(&mut self) -> Result<Option<ActiveWindow>, Self::Error>;

    /// Son klavye/fare girdisinden bu yana geçen saniye.
    fn idle_seconds(&mut self) -> Result<u64, Self::Error>;
}
