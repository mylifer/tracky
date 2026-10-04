use serde::{Deserialize, Serialize};

use crate::browser;
use crate::model::ActiveWindow;

/// Gizli pencereler ve başlığı gizlenen uygulamalar için kaydedilen başlık.
pub const HIDDEN_TITLE: &str = "Gizli";

/// Kullanıcının kontrolündeki gizlilik ayarları.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// Takip tamamen duraklatıldı.
    pub paused: bool,
    /// Bu uygulamalarda geçen süre hiç kaydedilmez (örn. şifre yöneticileri).
    pub excluded_apps: Vec<String>,
    /// Bu uygulamalarda süre kaydedilir ama pencere başlığı kaydedilmez.
    pub hidden_title_apps: Vec<String>,
    /// Tarayıcıların gizli pencerelerinde başlığı kaydetme.
    pub hide_private_windows: bool,
    /// Başlıkların sonundan silinen ekler (örn. Firefox profil adı " — Kaan").
    pub title_suffixes: Vec<String>,
    /// Bilgisayardan uzakta geçen süre (boşta, uyku) takvimde "Boşta" olarak kaydedilir.
    pub record_idle: bool,
    /// Bundan uzun boşluklar (gece gibi) kaydedilmez (dakika).
    pub idle_max_minutes: u32,
}

/// Varsayılan en uzun boşta kaydı: öğle arası ve uzun bir toplantı sığar, gece sığmaz.
pub const DEFAULT_IDLE_MAX_MINUTES: u32 = 180;

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            paused: false,
            excluded_apps: Vec::new(),
            hidden_title_apps: Vec::new(),
            hide_private_windows: true,
            title_suffixes: Vec::new(),
            record_idle: true,
            idle_max_minutes: DEFAULT_IDLE_MAX_MINUTES,
        }
    }
}

impl PrivacySettings {
    /// Gözlemi kayda hazırlar: gizlilik kurallarını uygular, tarayıcı başlığını temizler.
    /// `None` = bu an kaydedilmeyecek.
    pub fn apply(&self, mut window: ActiveWindow) -> Option<ActiveWindow> {
        if self.paused || contains(&self.excluded_apps, &window.app_id) {
            return None;
        }
        let is_browser = browser::is_browser(&window.app_id);
        let hide = contains(&self.hidden_title_apps, &window.app_id)
            || (is_browser
                && self.hide_private_windows
                && browser::is_private_window(&window.title));
        if hide {
            window.title = HIDDEN_TITLE.to_string();
            window.url = None;
        } else if is_browser {
            window.title = browser::clean_title(&window.title);
        }
        // Tarayıcı ekleri (Edge'in sıfır genişlikli boşluğu) temizlendikten sonra.
        window.app_name = strip_invisible(&window.app_name);
        window.title = strip_suffixes(&strip_invisible(&window.title), &self.title_suffixes);
        Some(window)
    }
}

/// Görünmez biçim karakterlerini (yazı yönü işaretleri, sıfır genişlikli boşluklar) atar.
/// Örn. WhatsApp adının başına U+200E ekler; aynı uygulama iki farklı ad gibi görünmesin.
fn strip_invisible(s: &str) -> String {
    s.chars()
        .filter(|c| {
            // Kontrol karakterleri de: PostgreSQL metinde NUL kabul etmez.
            !c.is_control()
                && !matches!(c,
                    '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Kullanıcının tanımladığı son ekleri (bir kez, ilk eşleşen) atar.
fn strip_suffixes(title: &str, suffixes: &[String]) -> String {
    for suffix in suffixes.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if let Some(rest) = title.strip_suffix(suffix) {
            let rest = rest.trim_end_matches([' ', '-', '—', '–', '|', '·']);
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    title.to_string()
}

impl PrivacySettings {
    /// Boşta kaydının en uzun süresi; kapalıysa `None`.
    pub fn max_away(&self) -> Option<chrono::Duration> {
        self.record_idle
            .then(|| chrono::Duration::minutes(i64::from(self.idle_max_minutes.clamp(15, 24 * 60))))
    }

    /// Uygulamanın pencere başlığına hiç ihtiyaç var mı? Hariç tutulan ya da başlığı
    /// gizlenen uygulamalarda başlık okunmaz.
    pub fn reads_title(&self, app_id: &str) -> bool {
        !contains(&self.excluded_apps, app_id) && !contains(&self.hidden_title_apps, app_id)
    }
}

fn contains(list: &[String], app_id: &str) -> bool {
    list.iter().any(|a| a.eq_ignore_ascii_case(app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(app_id: &str, title: &str) -> ActiveWindow {
        ActiveWindow {
            app_id: app_id.into(),
            app_name: "x".into(),
            title: title.into(),
            url: Some("https://example.com".into()),
        }
    }

    #[test]
    fn excludes_apps_and_pause() {
        let s = PrivacySettings {
            excluded_apps: vec!["com.1password.1password".into()],
            ..Default::default()
        };
        assert_eq!(s.apply(w("com.1Password.1password", "Kasa")), None);
        assert!(s.apply(w("com.apple.Notes", "Not")).is_some());

        let paused = PrivacySettings {
            paused: true,
            ..Default::default()
        };
        assert_eq!(paused.apply(w("com.apple.Notes", "Not")), None);
    }

    #[test]
    fn hides_titles() {
        let s = PrivacySettings {
            hidden_title_apps: vec!["com.apple.mail".into()],
            ..Default::default()
        };
        let out = s.apply(w("com.apple.mail", "Maaş bordrosu")).unwrap();
        assert_eq!((out.title.as_str(), out.url), (HIDDEN_TITLE, None));

        let out = s
            .apply(w("chrome.exe", "Yeni Sekme - Google Chrome (Incognito)"))
            .unwrap();
        assert_eq!(out.title, HIDDEN_TITLE);

        let off = PrivacySettings {
            hide_private_windows: false,
            ..Default::default()
        };
        let out = off
            .apply(w("chrome.exe", "Sayfa (Incognito) - Google Chrome"))
            .unwrap();
        assert_eq!(out.title, "Sayfa (Incognito)");
    }

    #[test]
    fn cleans_browser_titles_only() {
        let s = PrivacySettings::default();
        assert_eq!(
            s.apply(w("com.google.Chrome", "Gmail - Google Chrome"))
                .unwrap()
                .title,
            "Gmail"
        );
        assert_eq!(
            s.apply(w("com.test.app", "a - Google Chrome"))
                .unwrap()
                .title,
            "a - Google Chrome"
        );
    }

    #[test]
    fn strips_invisible_characters() {
        let mut win = w("net.whatsapp.WhatsApp", "\u{200e}What\u{0}sApp");
        win.app_name = "\u{200e}WhatsApp".into();
        let out = PrivacySettings::default().apply(win).unwrap();
        assert_eq!(
            (out.app_name.as_str(), out.title.as_str()),
            ("WhatsApp", "WhatsApp")
        );
    }

    #[test]
    fn strips_user_suffixes() {
        let s = PrivacySettings {
            title_suffixes: vec!["Kaan".into()],
            ..Default::default()
        };
        let out = s
            .apply(w("org.mozilla.firefox", "Anasayfa / X — Kaan"))
            .unwrap();
        assert_eq!(out.title, "Anasayfa / X");
        // Başlığın tamamı ekse dokunulmaz.
        assert_eq!(
            s.apply(w("org.mozilla.firefox", "Kaan")).unwrap().title,
            "Kaan"
        );
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s: PrivacySettings = serde_json::from_str(r#"{"paused":true}"#).unwrap();
        assert!(s.paused && s.hide_private_windows);
    }
}
