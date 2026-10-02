use url::Url;

/// URL'den raporlamada kullanılacak domaini çıkarır (`www.` atılır).
///
/// Tarayıcıların adres çubuğundan okunan değer çoğu zaman şemasızdır
/// (`github.com/foo`), bu yüzden şema yoksa `https://` varsayılır.
/// `chrome://`, `about:` gibi dahili sayfalar için `None` döner.
pub fn domain_of(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = if raw.contains("://") {
        Url::parse(raw).ok()?
    } else if raw.starts_with("about:") || raw.starts_with("data:") || raw.starts_with("file:") {
        return None;
    } else {
        Url::parse(&format!("https://{raw}")).ok()?
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host.is_empty() {
        return None;
    }
    Some(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::domain_of;

    #[test]
    fn extracts_domains() {
        assert_eq!(
            domain_of("https://www.github.com/a/b?x=1").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            domain_of("github.com/mylifer").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            domain_of("http://localhost:3000/x").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            domain_of("Docs.Google.com").as_deref(),
            Some("docs.google.com")
        );
    }

    #[test]
    fn ignores_internal_pages() {
        assert_eq!(domain_of("chrome://settings"), None);
        assert_eq!(domain_of("about:blank"), None);
        assert_eq!(domain_of("file:///tmp/a.html"), None);
        assert_eq!(domain_of(""), None);
    }
}
