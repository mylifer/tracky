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
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            paused: false,
            excluded_apps: Vec::new(),
            hidden_title_apps: Vec::new(),
            hide_private_windows: true,
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
        Some(window)
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
    fn missing_fields_use_defaults() {
        let s: PrivacySettings = serde_json::from_str(r#"{"paused":true}"#).unwrap();
        assert!(s.paused && s.hide_private_windows);
    }
}
