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
    /// Kullanıcının elle verdiği kategori; varsa kurallardan önce gelir.
    #[serde(default)]
    pub category_id: Option<String>,
    /// Kullanıcının elle verdiği proje; varsa kurallardan önce gelir.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// Elle eklenen kayıtların uygulama kimliği öneki: `kum.manual/<ad>`. Her ad ayrı
/// bir "uygulama" sayılır; böylece aynı adlı kayıtlar bir kuralla kategorilenebilir.
pub const MANUAL_APP_ID: &str = "kum.manual";

/// Kullanıcının bilgisayardan uzakta geçirdiği süre (girdi yok ya da bilgisayar uykuda).
/// Takvimde "Boşta" olarak görünür; kullanıcı bir projeye ya da kategoriye atayana kadar
/// çalışma süresine sayılmaz.
pub const IDLE_APP_ID: &str = "kum.idle";
/// Boşta kaydının görünen adı.
pub const IDLE_NAME: &str = "Boşta";

impl Session {
    pub fn is_manual(&self) -> bool {
        self.app_id
            .strip_prefix(MANUAL_APP_ID)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// Boşta geçen süre kaydı (atanmış olsun ya da olmasın).
    pub fn is_idle(&self) -> bool {
        self.app_id == IDLE_APP_ID
    }

    /// Çalışma süresine sayılır mı? Boşta kaydı ancak kullanıcı ona bir kategori ya da proje
    /// verince sayılır (örn. bilgisayardan uzakta geçen bir toplantı).
    pub fn counts_as_work(&self) -> bool {
        !self.is_idle()
            || self.category_id.is_some()
            || self
                .project_id
                .as_deref()
                .is_some_and(|p| p != crate::classify::NO_PROJECT)
    }

    /// `[from, to)` aralığı için boşta kaydı.
    pub fn idle(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            id: Uuid::new_v4(),
            app_id: IDLE_APP_ID.into(),
            app_name: IDLE_NAME.into(),
            title: String::new(),
            url: None,
            domain: None,
            started_at: from,
            ended_at: to,
            category_id: None,
            project_id: None,
        }
    }
}

/// Birden çok bilgisayarın (eşitlenmiş) oturumlarını tek zaman çizelgesinde birleştirir:
/// aynı ana düşen süre bir kez sayılır. İnsan aynı anda iki bilgisayarda çalışmaz; iki cihaz
/// aynı dakikalarda kayıt tuttuysa (örn. birinde video açık kaldı) o an en son başlayan
/// oturuma, yani en son geçilen bilgisayara yazılır. Önceki oturum araya giren kısım kadar
/// bölünür; parçalar aynı oturumun alanlarını taşır. Boşta kayıtları her zaman geri planda
/// kalır: bir cihaz boştayken diğerinde çalışılan süre çalışma sayılır. Girdi başlangıca göre
/// sıralı olmalı.
pub fn merge_devices(sessions: Vec<Session>) -> Vec<Session> {
    let (idle, active): (Vec<Session>, Vec<Session>) =
        sessions.into_iter().partition(Session::is_idle);
    let mut out = latest_wins(&active, &[]);
    out.extend(latest_wins(&idle, &out));
    out.sort_by_key(|s| s.started_at);
    out
}

/// Çakışan oturumlarda en son başlayan kazanır; `above` içindekiler her zaman kazanır.
/// Girdi başlangıca göre sıralı olmalı.
fn latest_wins(sessions: &[Session], above: &[Session]) -> Vec<Session> {
    let mut out = Vec::with_capacity(sessions.len());
    for (i, s) in sessions.iter().enumerate() {
        let mut pieces = vec![(s.started_at, s.ended_at)];
        let later = sessions[i + 1..]
            .iter()
            .take_while(|t| t.started_at < s.ended_at);
        let over = above
            .iter()
            .filter(|t| t.started_at < s.ended_at && t.ended_at > s.started_at);
        for cut in later.chain(over).map(|t| (t.started_at, t.ended_at)) {
            pieces = pieces
                .into_iter()
                .flat_map(|(a, b)| {
                    if cut.1 <= a || cut.0 >= b {
                        vec![(a, b)]
                    } else {
                        [(a, cut.0), (cut.1, b)]
                            .into_iter()
                            .filter(|(x, y)| y > x)
                            .collect()
                    }
                })
                .collect();
        }
        out.extend(pieces.into_iter().map(|(a, b)| Session {
            started_at: a,
            ended_at: b,
            ..s.clone()
        }));
    }
    out
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
            category_id: None,
            project_id: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn s(app: &str, from: i64, to: i64) -> Session {
        let t = |m: i64| {
            Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap() + chrono::Duration::minutes(m)
        };
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.into(),
            title: String::new(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
        }
    }

    fn spans(v: &[Session]) -> Vec<(String, i64)> {
        v.iter()
            .map(|x| (x.app_id.clone(), (x.ended_at - x.started_at).num_minutes()))
            .collect()
    }

    #[test]
    fn overlapping_devices_count_once_and_latest_switch_wins() {
        // Dizüstü 09:00–12:00 (video açık kaldı); masaüstünde 10:00–10:30 Figma, 10:30–11:00 Slack.
        let merged = merge_devices(vec![
            s("video", 0, 180),
            s("figma", 60, 90),
            s("slack", 90, 120),
        ]);
        assert_eq!(
            spans(&merged),
            [
                ("video".into(), 60),
                ("figma".into(), 30),
                ("slack".into(), 30),
                ("video".into(), 60),
            ]
        );
        let total: i64 = merged
            .iter()
            .map(|x| (x.ended_at - x.started_at).num_minutes())
            .sum();
        assert_eq!(total, 180);
        // Çakışma yoksa olduğu gibi kalır.
        let plain = vec![s("a", 0, 30), s("b", 30, 60)];
        assert_eq!(
            spans(&merge_devices(plain)),
            [("a".into(), 30), ("b".into(), 30)]
        );
    }

    #[test]
    fn idle_never_hides_work_on_another_device() {
        // Masaüstünde 09:00–12:00 kesintisiz çalışma; dizüstü 10:00–11:00 boşta (sonra başladı).
        let mut away = s(IDLE_APP_ID, 60, 120);
        away.app_name = IDLE_NAME.into();
        let merged = merge_devices(vec![s("code", 0, 180), away]);
        assert_eq!(spans(&merged), [("code".into(), 180)]);

        // Boşluk yalnızca çalışılmayan kısımda kalır.
        let merged = merge_devices(vec![s(IDLE_APP_ID, 0, 60), s("code", 20, 40)]);
        assert_eq!(
            spans(&merged),
            [
                (IDLE_APP_ID.into(), 20),
                ("code".into(), 20),
                (IDLE_APP_ID.into(), 20)
            ]
        );
    }

    #[test]
    fn idle_counts_as_work_only_when_assigned() {
        let mut away = Session::idle(Utc::now(), Utc::now());
        assert!(!away.counts_as_work());
        away.project_id = Some(crate::classify::NO_PROJECT.into());
        assert!(!away.counts_as_work());
        away.project_id = Some("kum".into());
        assert!(away.counts_as_work());
        away.project_id = None;
        away.category_id = Some("toplanti".into());
        assert!(away.counts_as_work());
        assert!(s("code", 0, 1).counts_as_work());
    }
}
