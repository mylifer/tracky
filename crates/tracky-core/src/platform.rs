use crate::model::ActiveWindow;

/// Her işletim sistemi için ayrı uygulanan gözlem arayüzü.
pub trait ActivityProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Ön plandaki pencere; ekran kilitliyse ya da pencere yoksa `None`.
    fn active_window(&mut self) -> Result<Option<ActiveWindow>, Self::Error>;

    /// `read_title(app_id)` yanlışsa pencere başlığı hiç okunmaz (boş kalır): kaydedilmeyen
    /// uygulamaların başlıkları belleğe bile alınmasın. Varsayılan: her zaman okur.
    fn active_window_with(
        &mut self,
        read_title: &dyn Fn(&str) -> bool,
    ) -> Result<Option<ActiveWindow>, Self::Error> {
        let _ = read_title;
        self.active_window()
    }

    /// Son klavye/fare girdisinden bu yana geçen saniye.
    fn idle_seconds(&mut self) -> Result<u64, Self::Error>;
}
