use chrono::{DateTime, Utc};

use crate::engine::{Engine, EngineConfig};
use crate::model::Session;
use crate::platform::ActivityProvider;
use crate::privacy::PrivacySettings;
use crate::store::Store;

/// Devam eden oturum bu kadar gözlemde bir diske yazılır (çökmede en fazla bu kadar saniye kaybolur).
const FLUSH_EVERY: u32 = 5;

/// Bir gözlemin sonucu.
#[derive(Debug, Default)]
pub struct TickOutcome {
    /// Devam eden oturum değişti (yeni pencere, boşta, duraklatma...).
    pub changed: bool,
    /// Gözlem ya da kayıt hatası; takip sürer.
    pub error: Option<String>,
}

/// Gözlem → gizlilik → motor → depolama hattı. Saniyede bir `tick` çağrılır.
pub struct Tracker<P: ActivityProvider> {
    provider: P,
    engine: Engine,
    privacy: PrivacySettings,
    ticks: u32,
}

impl<P: ActivityProvider> Tracker<P> {
    pub fn new(provider: P, config: EngineConfig, privacy: PrivacySettings) -> Self {
        Self {
            provider,
            engine: Engine::new(config),
            privacy,
            ticks: 0,
        }
    }

    pub fn privacy(&self) -> &PrivacySettings {
        &self.privacy
    }

    /// Yeni ayarlar bir sonraki gözlemden itibaren geçerli olur.
    pub fn set_privacy(&mut self, privacy: PrivacySettings) {
        self.privacy = privacy;
    }

    pub fn current(&self) -> Option<&Session> {
        self.engine.current()
    }

    pub fn tick(&mut self, store: &Store, now: DateTime<Utc>) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let window = match self.provider.active_window() {
            Ok(w) => w.and_then(|w| self.privacy.apply(w)),
            Err(e) => {
                outcome.error = Some(format!("pencere okunamadı: {e}"));
                None
            }
        };
        let idle = self.provider.idle_seconds().unwrap_or(0);
        let before = self.engine.current().map(|s| s.id);

        if let Some(closed) = self.engine.tick(now, window, idle) {
            save(store, &closed, &mut outcome);
        }
        outcome.changed = self.engine.current().map(|s| s.id) != before;

        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks.is_multiple_of(FLUSH_EVERY)
            && let Some(current) = self.engine.current()
        {
            save(store, current, &mut outcome);
        }
        outcome
    }

    /// Kapanışta devam eden oturumu kaydeder.
    pub fn shutdown(&mut self, store: &Store, now: DateTime<Utc>) -> Option<String> {
        let mut outcome = TickOutcome::default();
        if let Some(last) = self.engine.flush(now) {
            save(store, &last, &mut outcome);
        }
        outcome.error
    }
}

fn save(store: &Store, session: &Session, outcome: &mut TickOutcome) {
    if let Err(e) = store.upsert_session(session) {
        outcome.error = Some(format!("kayıt yazılamadı: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActiveWindow;
    use chrono::{Duration, TimeZone};
    use std::collections::VecDeque;

    #[derive(Debug, thiserror::Error)]
    #[error("test")]
    struct Never;

    /// Sırayla verilen gözlemleri döndüren sahte sağlayıcı.
    struct Script(VecDeque<Option<ActiveWindow>>);

    impl ActivityProvider for Script {
        type Error = Never;
        fn active_window(&mut self) -> Result<Option<ActiveWindow>, Never> {
            Ok(self.0.pop_front().flatten())
        }
        fn idle_seconds(&mut self) -> Result<u64, Never> {
            Ok(0)
        }
    }

    fn win(app: &str) -> Option<ActiveWindow> {
        Some(ActiveWindow {
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: "t".into(),
            url: None,
        })
    }

    #[test]
    fn records_sessions_and_respects_privacy() {
        let store = Store::open_in_memory().unwrap();
        let script = [
            win("A"),
            win("A"),
            win("Secret"),
            win("Secret"),
            win("B"),
            win("B"),
        ];
        let privacy = PrivacySettings {
            excluded_apps: vec!["com.test.Secret".into()],
            ..Default::default()
        };
        let mut tracker = Tracker::new(
            Script(script.into_iter().collect()),
            EngineConfig::default(),
            privacy,
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let changes: Vec<bool> = (0..6)
            .map(|i| tracker.tick(&store, t0 + Duration::seconds(i)).changed)
            .collect();
        assert_eq!(changes, [true, false, true, false, true, false]);
        assert!(
            tracker
                .shutdown(&store, t0 + Duration::seconds(6))
                .is_none()
        );

        let apps = store.app_totals(t0, t0 + Duration::hours(1)).unwrap();
        let got: Vec<_> = apps.iter().map(|u| (u.label.as_str(), u.seconds)).collect();
        assert_eq!(got, [("A", 2), ("B", 2)]);
    }
}
