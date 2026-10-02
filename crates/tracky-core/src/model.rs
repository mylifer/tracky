use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Platform katmanının bir anda gözlemlediği ön plandaki pencere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWindow {
    /// Uygulamanın kararlı kimliği: macOS'ta bundle id, Windows'ta exe yolu.
    pub app_id: String,
    /// Kullanıcıya gösterilecek uygulama adı.
    pub app_name: String,
    pub title: String,
    /// Tarayıcılarda aktif sekmenin URL'si (alınabildiyse).
    pub url: Option<String>,
}

/// Aynı pencerede kesintisiz geçirilen bir zaman aralığı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

impl Session {
    pub fn start(window: ActiveWindow, at: DateTime<Utc>) -> Self {
        let domain = window.url.as_deref().and_then(crate::url_util::domain_of);
        Self {
            id: Uuid::new_v4(),
            app_id: window.app_id,
            app_name: window.app_name,
            title: window.title,
            url: window.url,
            domain,
            started_at: at,
            ended_at: at,
        }
    }

    pub fn duration(&self) -> chrono::Duration {
        self.ended_at - self.started_at
    }

    /// Gözlemlenen pencere bu oturumun devamı mı?
    pub fn matches(&self, window: &ActiveWindow) -> bool {
        self.app_id == window.app_id && self.title == window.title && self.url == window.url
    }
}
