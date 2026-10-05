//! Sitenin simgesini (favicon) bulmak: ana sayfanın `<head>`'indeki `<link rel="icon">`
//! bağlantıları tercih sırasına dizilir. Ağ işi uygulamadadır; burası yalnızca ayrıştırır.

use url::Url;

/// Simgeler bu boyuta (px) yakın olsun: takvimde 16–20 px, Retina'da iki katı.
const TARGET_PX: u32 = 64;

/// Sayfadaki simge adresleri, en uygun olandan başlayarak. Göreli adresler `page`'e göre
/// çözülür; `data:image/…` adresleri olduğu gibi döner. Hiç bağlantı yoksa boş liste
/// (çağıran `/favicon.ico`'yu dener).
pub fn icon_links(html: &str, page: &Url) -> Vec<String> {
    // Bağlantılar başlıktadır; gövdedeki dev sayfayı taramaya gerek yok.
    let head = match find_ci(html, "</head") {
        Some(i) => &html[..i],
        None => html,
    };
    let mut found: Vec<(u32, String)> = Vec::new();
    let mut rest = head;
    while let Some(i) = find_ci(rest, "<link") {
        rest = &rest[i + 5..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        rest = &rest[end..];
        let attrs = attributes(tag);
        let get = |name: &str| {
            attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        };
        let rel = get("rel").unwrap_or("").to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_ascii_whitespace().collect();
        // `mask-icon` tek renkli bir şablondur; renkli simge yerine geçmez.
        let touch = rels.iter().any(|r| r.starts_with("apple-touch-icon"));
        if !touch && !rels.contains(&"icon") {
            continue;
        }
        let Some(href) = get("href").map(str::trim).filter(|h| !h.is_empty()) else {
            continue;
        };
        let href = if href.starts_with("data:") {
            if !href.starts_with("data:image/") {
                continue;
            }
            href.to_string()
        } else {
            match page.join(href) {
                Ok(u) if matches!(u.scheme(), "http" | "https") => u.to_string(),
                _ => continue,
            }
        };
        let svg = get("type").is_some_and(|t| t.eq_ignore_ascii_case("image/svg+xml"))
            || href.starts_with("data:image/svg")
            || href
                .split(['?', '#'])
                .next()
                .is_some_and(|p| p.to_ascii_lowercase().ends_with(".svg"));
        found.push((rank(svg, touch, get("sizes")), href));
    }
    found.sort_by_key(|(rank, _)| *rank);
    let mut out: Vec<String> = Vec::new();
    for (_, href) in found {
        if !out.contains(&href) {
            out.push(href);
        }
    }
    out
}

/// Küçük sayı önce: ölçeklenen SVG, sonra hedefe yakın PNG, boyutu bilinmeyen simge,
/// dokunmatik simge (genelde dolu zeminli, büyük), en son 16 px'lik simgeler.
fn rank(svg: bool, touch: bool, sizes: Option<&str>) -> u32 {
    if svg {
        return 0;
    }
    let largest = sizes.and_then(|s| {
        s.split_ascii_whitespace()
            .filter_map(|wh| wh.split(['x', 'X']).next()?.parse::<u32>().ok())
            .max()
    });
    match largest {
        _ if touch => 3000,
        Some(px) if px >= 32 => 1000 + px.abs_diff(TARGET_PX),
        Some(_) => 4000,
        None => 2000,
    }
}

/// `ad="değer"`, `ad='değer'`, `ad=değer` ve değersiz öznitelikler.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let s = tag.trim_end_matches('/');
    let mut chars = s.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() || c == '/' {
            chars.next();
            continue;
        }
        let mut name_end = start;
        while let Some(&(i, c)) = chars.peek() {
            if c.is_whitespace() || c == '=' {
                break;
            }
            name_end = i + c.len_utf8();
            chars.next();
        }
        let name = s[start..name_end].to_string();
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none_or(|&(_, c)| c != '=') {
            out.push((name, String::new()));
            continue;
        }
        chars.next();
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let value = match chars.peek() {
            Some(&(i, q @ ('"' | '\''))) => {
                chars.next();
                let from = i + 1;
                let mut to = s.len();
                for (j, c) in chars.by_ref() {
                    if c == q {
                        to = j;
                        break;
                    }
                }
                s[from..to.max(from)].to_string()
            }
            Some(&(i, _)) => {
                let mut to = s.len();
                while let Some(&(j, c)) = chars.peek() {
                    if c.is_whitespace() {
                        to = j;
                        break;
                    }
                    chars.next();
                }
                s[i..to].to_string()
            }
            None => String::new(),
        };
        out.push((name, html_unescape(&value)));
    }
    out
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Büyük/küçük harfe duyarsız ASCII arama (bayt konumu).
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    hay.as_bytes()
        .windows(n)
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Ana makine adı ağa sorulabilecek gibi mi: harf, rakam, `-`, `.` ve isteğe bağlı port.
/// Kayıtlardaki alan adı dışarıdan (eşitleme) de gelebilir; adrese olduğu gibi yazılır.
pub fn is_fetchable_host(host: &str) -> bool {
    let (name, port) = host.split_once(':').unwrap_or((host, ""));
    !name.is_empty()
        && name.len() <= 253
        && (name.contains('.') || name == "localhost")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && port.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(html: &str) -> Vec<String> {
        icon_links(html, &Url::parse("https://site.com/").unwrap())
    }

    #[test]
    fn prefers_svg_then_sizes_near_target() {
        let html = r#"<html><head>
            <link rel="shortcut icon" href="/favicon.ico">
            <link rel="icon" sizes="16x16" href="/16.png">
            <link rel="icon" sizes="32x32" href="/32.png">
            <link rel="icon" sizes="192x192" href="/192.png">
            <link rel="apple-touch-icon" href="/touch.png">
            <link rel="icon" type="image/svg+xml" href="/icon.svg">
            <link rel="mask-icon" href="/mask.svg">
            <link rel="stylesheet" href="/a.css">
        </head><body><link rel="icon" href="/body.png"></body></html>"#;
        assert_eq!(
            links(html),
            [
                "https://site.com/icon.svg",
                "https://site.com/32.png",
                "https://site.com/192.png",
                "https://site.com/favicon.ico",
                "https://site.com/touch.png",
                "https://site.com/16.png",
            ]
        );
    }

    #[test]
    fn resolves_relative_and_keeps_data_urls() {
        let page = Url::parse("https://a.site.com/app/").unwrap();
        let html = r#"<LINK REL=icon HREF=img/x.png><link href='//cdn.com/y.png' rel='icon'/>
            <link rel="icon" href="data:image/svg+xml,%3Csvg%3E">
            <link rel="icon" href="data:text/html,x"><link rel="icon" href="javascript:x">"#;
        let mut got = icon_links(html, &page);
        got.sort();
        assert_eq!(
            got,
            [
                "data:image/svg+xml,%3Csvg%3E",
                "https://a.site.com/app/img/x.png",
                "https://cdn.com/y.png",
            ]
        );
    }

    #[test]
    fn unescapes_and_dedupes() {
        let html =
            r#"<link rel="icon" href="/i.png?a=1&amp;b=2"><link rel="icon" href="/i.png?a=1&b=2">"#;
        assert_eq!(links(html), ["https://site.com/i.png?a=1&b=2"]);
    }

    #[test]
    fn host_check() {
        assert!(is_fetchable_host("github.com"));
        assert!(is_fetchable_host("intranet.firma.local:8080"));
        assert!(is_fetchable_host("localhost:3000"));
        assert!(!is_fetchable_host("site"));
        assert!(!is_fetchable_host("a.com/../x"));
        assert!(!is_fetchable_host("a.com@b.com"));
        assert!(!is_fetchable_host(""));
    }
}
