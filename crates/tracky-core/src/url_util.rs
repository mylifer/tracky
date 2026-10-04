use url::Url;

/// Kayda girecek URL: sorgu (`?…`) ve parça (`#…`) atılır; bunlar çoğu zaman oturum
/// anahtarı, arama ya da kişisel veri taşır ve sınıflandırmaya bir şey katmaz. Yalnızca
/// http(s) adresleri kalır.
pub fn sanitize(raw: &str) -> Option<String> {
    let raw = raw.trim();
    domain_of(raw)?;
    let mut url = if raw.contains("://") {
        Url::parse(raw).ok()?
    } else {
        Url::parse(&format!("https://{raw}")).ok()?
    };
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some(url.to_string())
}

/// Alan adı kuralları için URL'nin `alan/yol` biçimi (küçük harf, `www.` ve şema yok,
/// sondaki `/` yok): `https://www.GitHub.com/Firma/` → `github.com/firma`.
///
/// Yol çözülür (`%C5%9F` → `ş`) ve [`crate::search::fold`] ile katlanır: kayıtlı adresler
/// [`sanitize`] sonrası kodlanmış, kullanıcının yazdığı desen ise çoğu zaman ham olur.
pub fn host_path(raw: &str) -> Option<String> {
    let domain = domain_of(raw)?;
    let raw = raw.trim();
    let rest = raw.split_once("://").map_or(raw, |(_, r)| r);
    let path = rest
        .find('/')
        .map_or("", |i| &rest[i..])
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let path = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    let path = crate::search::fold(path.trim_end_matches('/'));
    Some(format!("{domain}{path}"))
}

/// Alan adı kuralı deseni, eşleşen `alan/yol` (bkz. [`host_path`]) ile karşılaştırılır:
/// `github.com` hem `github.com/...` hem `gist.github.com` adreslerine uyar,
/// `github.com/firma` yalnızca o yolun altındakilere. İkisi de küçük harfli olmalı.
pub fn pattern_matches(pattern: &str, host_path: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    if pattern.is_empty() {
        return false;
    }
    let (p_host, p_path) = pattern
        .split_once('/')
        .map_or((pattern, ""), |(h, p)| (h, p));
    let (host, path) = host_path
        .split_once('/')
        .map_or((host_path, ""), |(h, p)| (h, p));
    let host_ok = host == p_host || host.ends_with(&format!(".{p_host}"));
    let path_ok = p_path.is_empty() || path == p_path || path.starts_with(&format!("{p_path}/"));
    host_ok && path_ok
}

/// Kullanıcının yazdığı alan adı desenini düzgünleştirir: şema, `www.`, sorgu ve sondaki
/// `/` atılır, küçük harfe çevrilir. Adres değilse `None`.
pub fn normalize_pattern(raw: &str) -> Option<String> {
    host_path(raw.trim().trim_start_matches("*."))
}

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
    use super::*;

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
    fn sanitizes_urls_and_matches_patterns() {
        assert_eq!(
            sanitize("https://user:pw@mail.google.com/mail/u/0/?tab=rm#inbox/123").as_deref(),
            Some("https://mail.google.com/mail/u/0/")
        );
        assert_eq!(
            sanitize("github.com/firma/repo?token=x").as_deref(),
            Some("https://github.com/firma/repo")
        );
        assert_eq!(sanitize("chrome://settings"), None);
        assert_eq!(sanitize("Google'da ara"), None);

        assert_eq!(
            host_path("https://www.GitHub.com/Firma/Repo/?x=1").as_deref(),
            Some("github.com/firma/repo")
        );
        assert_eq!(host_path("github.com").as_deref(), Some("github.com"));
        assert_eq!(
            normalize_pattern(" https://www.Jira.togg.com/ ").as_deref(),
            Some("jira.togg.com")
        );
        assert_eq!(normalize_pattern("*.togg.com").as_deref(), Some("togg.com"));

        assert!(pattern_matches("github.com", "github.com/firma/repo"));
        assert!(pattern_matches("github.com", "gist.github.com"));
        assert!(!pattern_matches("github.com", "notgithub.com"));
        assert!(pattern_matches("github.com/firma", "github.com/firma/repo"));
        assert!(pattern_matches("github.com/firma", "github.com/firma"));
        assert!(!pattern_matches("github.com/firma", "github.com/firmab"));
        assert!(!pattern_matches("github.com/firma", "github.com"));
        assert!(!pattern_matches("", "github.com"));
    }

    #[test]
    fn non_ascii_paths_match_encoded_urls() {
        let stored = sanitize("https://tr.wikipedia.org/wiki/İstanbul_Boğazı?x=1").unwrap();
        assert!(stored.contains("%C4%B0"));
        let pattern = normalize_pattern("tr.wikipedia.org/wiki/istanbul_boğazı").unwrap();
        assert!(pattern_matches(&pattern, &host_path(&stored).unwrap()));
        // Kodlanmış yazılmış desen de aynı biçime iner.
        assert_eq!(
            normalize_pattern("tr.wikipedia.org/wiki/%C4%B0stanbul_Bo%C4%9Faz%C4%B1"),
            Some(pattern)
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
