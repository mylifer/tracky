use chrono::{DateTime, Duration, Utc};

use crate::model::{ActiveWindow, Session};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Bu kadar süre girdi yoksa kullanıcı boşta sayılır.
    pub idle_threshold: Duration,
    /// İki gözlem arası bundan uzunsa (uyku, askıya alma) oturum son gözlemde kapatılır.
    pub max_gap: Duration,
    /// Bundan kısa oturumlar (hızlı alt-tab geçişleri) kaydedilmez.
    pub min_session: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            idle_threshold: Duration::minutes(3),
            max_gap: Duration::seconds(15),
            min_session: Duration::seconds(1),
        }
    }
}

/// Periyodik gözlemleri (örn. saniyede bir) oturumlara dönüştürür.
///
/// Motor saf mantıktır: zamanı ve gözlemleri dışarıdan alır, kapanan
/// oturumları döndürür. Kalıcılık çağıranın sorumluluğundadır.
#[derive(Debug)]
pub struct Engine {
    config: EngineConfig,
    current: Option<Session>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            current: None,
        }
    }

    /// Devam eden oturum; çökme durumunda kayıp olmaması için periyodik upsert edilebilir.
    pub fn current(&self) -> Option<&Session> {
        self.current.as_ref()
    }

    /// Bir gözlemi işler ve bu gözlemle kapanan oturumu (varsa) döndürür.
    pub fn tick(
        &mut self,
        now: DateTime<Utc>,
        window: Option<ActiveWindow>,
        idle_seconds: u64,
    ) -> Option<Session> {
        // Makine uyuduysa son görülen andan sonrası sayılmaz.
        let slept = self
            .current
            .as_ref()
            .is_some_and(|s| now - s.ended_at > self.config.max_gap);
        let closed = if slept { self.close_at(None) } else { None };
        // Oturum yukarıda kapandıysa `observe` ikinci bir oturum kapatamaz.
        let switched = self.observe(now, window, idle_seconds);
        closed.or(switched)
    }

    /// Uygulama kapanırken devam eden oturumu kapatır.
    pub fn flush(&mut self, now: DateTime<Utc>) -> Option<Session> {
        self.close_at(Some(now))
    }

    fn observe(
        &mut self,
        now: DateTime<Utc>,
        window: Option<ActiveWindow>,
        idle_seconds: u64,
    ) -> Option<Session> {
        // Platform anlamsız büyük bir değer döndürebilir (macOS'ta +inf → u64::MAX);
        // `Duration::seconds` ve `now - idle` taşmasın diye bir yılla sınırlanır.
        const MAX_IDLE_SECONDS: u64 = 365 * 24 * 3600;
        let idle = Duration::seconds(idle_seconds.min(MAX_IDLE_SECONDS) as i64);
        if idle >= self.config.idle_threshold {
            // Boşluk son girdiden itibaren başlar; o ana kadar kullanıcı oradaydı.
            return self.close_at(Some(now - idle));
        }
        let Some(window) = window else {
            return self.close_at(Some(now));
        };
        match &mut self.current {
            Some(s) if s.matches(&window) => {
                // Sistem saati geri alınırsa bitiş başlangıçtan önceye düşmesin.
                s.ended_at = now.max(s.started_at);
                None
            }
            _ => {
                let closed = self.close_at(Some(now));
                self.current = Some(Session::start(window, now));
                closed
            }
        }
    }

    /// Devam eden oturumu `at` anında kapatır (`None` = son görülen an).
    /// Bitiş hiçbir zaman başlangıçtan önce olamaz.
    fn close_at(&mut self, at: Option<DateTime<Utc>>) -> Option<Session> {
        let mut s = self.current.take()?;
        if let Some(at) = at {
            s.ended_at = at.max(s.started_at);
        }
        (s.duration() >= self.config.min_session).then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn win(app: &str, title: &str) -> Option<ActiveWindow> {
        Some(ActiveWindow {
            app_id: format!("com.test.{app}"),
            app_name: app.to_string(),
            title: title.to_string(),
            url: None,
        })
    }

    fn run(engine: &mut Engine, steps: &[(i64, Option<ActiveWindow>, u64)]) -> Vec<Session> {
        steps
            .iter()
            .filter_map(|(at, w, idle)| engine.tick(t(*at), w.clone(), *idle))
            .collect()
    }

    #[test]
    fn merges_consecutive_samples_and_splits_on_change() {
        let mut e = Engine::new(EngineConfig::default());
        let closed = run(
            &mut e,
            &[
                (0, win("Code", "main.rs"), 0),
                (1, win("Code", "main.rs"), 0),
                (2, win("Code", "main.rs"), 0),
                (3, win("Safari", "GitHub"), 0),
                (4, win("Safari", "GitHub"), 0),
            ],
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].app_name, "Code");
        assert_eq!(closed[0].started_at, t(0));
        assert_eq!(closed[0].ended_at, t(3));
        let cur = e.current().unwrap();
        assert_eq!(
            (cur.app_name.as_str(), cur.started_at, cur.ended_at),
            ("Safari", t(3), t(4))
        );
    }

    #[test]
    fn title_change_starts_new_session() {
        let mut e = Engine::new(EngineConfig::default());
        let closed = run(
            &mut e,
            &[(0, win("Code", "a.rs"), 0), (5, win("Code", "b.rs"), 0)],
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].title, "a.rs");
    }

    #[test]
    fn idle_closes_session_at_last_input() {
        let mut e = Engine::new(EngineConfig::default());
        let mut steps: Vec<_> = (0..=10).map(|s| (s, win("Code", "x"), 0)).collect();
        // 10. saniyeden sonra girdi yok; 190. saniyede idle = 180s eşiğe ulaşır.
        steps.extend((11..=190).map(|s| (s, win("Code", "x"), (s - 10) as u64)));
        let closed = run(&mut e, &steps);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].ended_at, t(10));
        assert!(e.current().is_none());

        // Kullanıcı dönünce yeni oturum başlar.
        assert!(e.tick(t(200), win("Code", "x"), 0).is_none());
        assert_eq!(e.current().unwrap().started_at, t(200));
    }

    #[test]
    fn sleep_gap_closes_session_at_last_seen() {
        let mut e = Engine::new(EngineConfig::default());
        let closed = run(
            &mut e,
            &[
                (0, win("Code", "x"), 0),
                (5, win("Code", "x"), 0),
                (3600, win("Code", "x"), 0),
            ],
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].ended_at, t(5));
        assert_eq!(e.current().unwrap().started_at, t(3600));
    }

    #[test]
    fn drops_too_short_sessions() {
        let mut e = Engine::new(EngineConfig {
            min_session: Duration::seconds(2),
            ..Default::default()
        });
        let closed = run(
            &mut e,
            &[
                (0, win("A", "a"), 0),
                (1, win("B", "b"), 0),
                (4, win("C", "c"), 0),
            ],
        );
        assert_eq!(
            closed
                .iter()
                .map(|s| s.app_name.as_str())
                .collect::<Vec<_>>(),
            ["B"]
        );
    }

    #[test]
    fn no_window_closes_and_flush_closes() {
        let mut e = Engine::new(EngineConfig::default());
        let closed = run(
            &mut e,
            &[(0, win("A", "a"), 0), (4, None, 0), (6, win("B", "b"), 0)],
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].ended_at, t(4));
        let flushed = e.flush(t(9)).unwrap();
        assert_eq!((flushed.started_at, flushed.ended_at), (t(6), t(9)));
        assert!(e.current().is_none());
    }

    #[test]
    fn extracts_domain_from_url() {
        let mut e = Engine::new(EngineConfig::default());
        let mut w = win("Chrome", "Repo").unwrap();
        w.url = Some("https://www.github.com/mylifer/tracky".into());
        e.tick(t(0), Some(w), 0);
        assert_eq!(e.current().unwrap().domain.as_deref(), Some("github.com"));
    }
}
