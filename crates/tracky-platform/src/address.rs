//! Tarayıcı adresi okumanın platformdan bağımsız kısmı: önbellek ve adres çubuğu metninin
//! adres olup olmadığına karar verme.

use std::time::{Duration, Instant};

/// Aynı pencerede adres en çok bu sıklıkla yeniden okunur.
const REFRESH: Duration = Duration::from_secs(15);

/// Adres her gözlemde okunmaz: erişilebilirlik ağacında gezinmek saniyelik takibe göre
/// pahalıdır. Aynı uygulama ve başlık için `REFRESH` boyunca önceki sonuç kullanılır.
#[derive(Debug, Default)]
pub(crate) struct AddressCache {
    last: Option<Entry>,
}

#[derive(Debug)]
struct Entry {
    app_id: String,
    title: String,
    url: Option<String>,
    read_at: Instant,
}

impl AddressCache {
    pub(crate) fn get(
        &mut self,
        app_id: &str,
        title: &str,
        read: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let same = self
            .last
            .as_ref()
            .filter(|e| e.app_id == app_id && e.title == title);
        if let Some(e) = same
            && e.read_at.elapsed() < REFRESH
        {
            return e.url.clone();
        }
        // Aynı pencerede adres bir an okunamazsa (adres çubuğuna yazılıyor, sayfa yükleniyor)
        // önceki adres kalır; yoksa oturum her yenilemede bölünürdü.
        let url = read().or_else(|| same.and_then(|e| e.url.clone()));
        self.last = Some(Entry {
            app_id: app_id.to_string(),
            title: title.to_string(),
            url: url.clone(),
            read_at: Instant::now(),
        });
        url
    }
}

/// Adres çubuğundaki metin bir web adresi mi? (Yazılmakta olan arama metni değil.)
pub(crate) fn looks_like_address(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.contains(char::is_whitespace)
        && (text.contains('.') || text.contains("://") || text.starts_with("localhost"))
        && tracky_core::url_util::domain_of(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_per_window_and_keeps_the_last_address() {
        let mut cache = AddressCache::default();
        let mut reads = 0;
        let mut read = |url: Option<&str>| {
            reads += 1;
            url.map(str::to_string)
        };
        assert_eq!(
            cache.get("chrome", "A", || read(Some("a.com"))).as_deref(),
            Some("a.com")
        );
        assert_eq!(
            cache.get("chrome", "A", || read(Some("b.com"))).as_deref(),
            Some("a.com")
        );
        assert_eq!(cache.get("chrome", "B", || read(None)), None);
        assert_eq!(reads, 2);
    }

    #[test]
    fn recognizes_addresses() {
        assert!(looks_like_address("github.com/mylifer/tracky"));
        assert!(looks_like_address("https://mail.google.com/mail/u/0/"));
        assert!(looks_like_address("localhost:3000"));
        assert!(!looks_like_address("rust borrow checker"));
        assert!(!looks_like_address("kum"));
        assert!(!looks_like_address(""));
    }
}
