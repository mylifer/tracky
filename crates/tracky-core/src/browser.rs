//! Tarayıcı pencereleri: tanıma, başlık temizleme ve gizli pencere tespiti.
//!
//! Sınıflandırma çoğunlukla pencere başlığına (= aktif sekmenin adı) dayanır. Adres de
//! okunabildiğinde (macOS'ta Erişilebilirlik, Windows'ta UI Automation; Otomasyon izni
//! gerekmez) web sitesi kuralları için kullanılır; gizli pencerelerde kaydedilmez.

/// macOS bundle id'leri.
const MAC_BUNDLES: &[&str] = &[
    "com.google.chrome",
    "com.google.chrome.canary",
    "com.apple.safari",
    "com.apple.safaritechnologypreview",
    "com.microsoft.edgemac",
    "company.thebrowser.browser", // Arc
    "com.brave.browser",
    "org.mozilla.firefox",
    "org.mozilla.firefoxdeveloperedition",
    "com.operasoftware.opera",
    "com.vivaldi.vivaldi",
    "app.zen-browser.zen",
];

/// Windows exe adları.
const WIN_EXES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
    "zen.exe",
];

/// Tarayıcıların pencere başlığına eklediği son ekler (uzundan kısaya).
const TITLE_SUFFIXES: &[&str] = &[
    " - Google Chrome",
    " - Microsoft\u{200b} Edge", // Edge sıfır genişlikli boşluk kullanır
    " - Microsoft Edge",
    " — Mozilla Firefox",
    " - Mozilla Firefox",
    " — Firefox Developer Edition",
    " - Brave",
    " - Opera",
    " - Vivaldi",
    " — Zen Browser",
];

/// Gizli pencere işaretleri (küçük harf). Başlık bazlı olduğu için en iyi çaba esaslıdır.
const PRIVATE_MARKERS: &[&str] = &[
    "incognito",
    "inprivate",
    "private browsing",
    "gizli gezinti",
    "(gizli)",
    "- gizli",
];

/// `app_id` bir tarayıcıya mı ait? (macOS bundle id ya da Windows exe yolu)
pub fn is_browser(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    if MAC_BUNDLES.contains(&id.as_str()) {
        return true;
    }
    let exe = id.rsplit(['\\', '/']).next().unwrap_or(&id);
    WIN_EXES.contains(&exe)
}

/// Başlıktan tarayıcı adını ve Edge'in "ve N sayfa daha" ekini atar.
pub fn clean_title(title: &str) -> String {
    let mut t = title.trim();
    for suffix in TITLE_SUFFIXES {
        if let Some(rest) = t.strip_suffix(suffix) {
            t = rest;
            break;
        }
    }
    // Edge: "Sayfa and 3 more pages" / "Sayfa ve 3 sayfa daha"
    for (sep, tail) in [(" and ", " more page"), (" ve ", " sayfa daha")] {
        if let Some(idx) = t.rfind(sep) {
            let rest = &t[idx + sep.len()..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 && rest[digits..].starts_with(tail) {
                t = &t[..idx];
            }
        }
    }
    t.trim().to_string()
}

pub fn is_private_window(title: &str) -> bool {
    let t = title.to_lowercase();
    PRIVATE_MARKERS.iter().any(|m| t.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_browsers_on_both_platforms() {
        assert!(is_browser("com.google.Chrome"));
        assert!(is_browser("company.thebrowser.Browser"));
        assert!(is_browser(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        ));
        assert!(is_browser(
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
        ));
        assert!(!is_browser("com.microsoft.VSCode"));
        assert!(!is_browser(r"C:\Windows\explorer.exe"));
    }

    #[test]
    fn strips_browser_suffixes() {
        assert_eq!(
            clean_title("Inbox (3) - Gmail - Google Chrome"),
            "Inbox (3) - Gmail"
        );
        assert_eq!(clean_title("GitHub — Mozilla Firefox"), "GitHub");
        assert_eq!(
            clean_title("Docs and 4 more pages - Kişisel - Microsoft\u{200b} Edge"),
            "Docs"
        );
        assert_eq!(
            clean_title("Docs and 4 more pages - Microsoft Edge"),
            "Docs"
        );
        assert_eq!(clean_title("Belge ve 2 sayfa daha"), "Belge");
        assert_eq!(clean_title("Safari sayfası"), "Safari sayfası");
        assert_eq!(clean_title("Rock and Roll"), "Rock and Roll");
    }

    #[test]
    fn detects_private_windows() {
        assert!(is_private_window("New Tab - Google Chrome (Incognito)"));
        assert!(is_private_window("InPrivate - Microsoft Edge"));
        assert!(is_private_window("Mozilla Firefox Private Browsing"));
        assert!(is_private_window("Yeni Sekme - Google Chrome (Gizli)"));
        assert!(!is_private_window("Gmail - Google Chrome"));
    }
}
