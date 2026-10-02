use chrono::{DateTime, Utc};

use uuid::Uuid;

use crate::engine::{Engine, EngineConfig};
use crate::model::{ActiveWindow, Session};
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

/// Platformdan okunan ham an (depoya dokunmadan alınır).
#[derive(Debug)]
pub struct Observation {
    window: Option<ActiveWindow>,
    idle: u64,
    error: Option<String>,
}

/// Gözlem → gizlilik → motor → depolama hattı. Saniyede bir `tick` çağrılır.
///
/// Platform çağrıları (macOS'ta yanıt vermeyen uygulamada saniyeler sürebilir)
/// depo kilidi tutulmadan yapılabilsin diye `observe` ve `record` ayrıdır.
pub struct Tracker<P: ActivityProvider> {
    provider: P,
    engine: Engine,
    privacy: PrivacySettings,
    ticks: u32,
    /// Diske yazılmış, hâlâ devam eden oturum.
    flushed: Option<Uuid>,
}

impl<P: ActivityProvider> Tracker<P> {
    pub fn new(provider: P, config: EngineConfig, privacy: PrivacySettings) -> Self {
        Self {
            provider,
            engine: Engine::new(config),
            privacy,
            ticks: 0,
            flushed: None,
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
        let observation = self.observe();
        self.record(store, now, observation)
    }

    pub fn observe(&mut self) -> Observation {
        let (window, error) = match self.provider.active_window() {
            Ok(w) => (w.and_then(|w| self.privacy.apply(w)), None),
            Err(e) => (None, Some(format!("pencere okunamadı: {e}"))),
        };
        Observation {
            window,
            idle: self.provider.idle_seconds().unwrap_or(0),
            error,
        }
    }

    pub fn record(&mut self, store: &Store, now: DateTime<Utc>, obs: Observation) -> TickOutcome {
        let mut outcome = TickOutcome {
            changed: false,
            error: obs.error,
        };
        let before = self.engine.current().map(|s| s.id);

        let closed = self.engine.tick(now, obs.window, obs.idle);
        self.settle(store, before, closed, &mut outcome);
        outcome.changed = self.engine.current().map(|s| s.id) != before;

        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks.is_multiple_of(FLUSH_EVERY)
            && let Some(current) = self.engine.current()
        {
            save(store, current, &mut outcome);
            self.flushed = Some(current.id);
        }
        outcome
    }

    /// Kapanışta devam eden oturumu kaydeder.
    pub fn shutdown(&mut self, store: &Store, now: DateTime<Utc>) -> Option<String> {
        let mut outcome = TickOutcome::default();
        let before = self.engine.current().map(|s| s.id);
        let closed = self.engine.flush(now);
        self.settle(store, before, closed, &mut outcome);
        outcome.error
    }

    /// Kapanan oturumu yazar. Motor oturumu çok kısa diye attıysa ama daha önce
    /// diske yazılmışsa (örn. boşta kalınca bitiş geriye çekildi), kaydı sil;
    /// yoksa eski, şişkin bitiş zamanı raporda kalırdı.
    fn settle(
        &mut self,
        store: &Store,
        before: Option<Uuid>,
        closed: Option<Session>,
        outcome: &mut TickOutcome,
    ) {
        let still_running = self.engine.current().map(|s| s.id) == before;
        match closed {
            Some(closed) => save(store, &closed, outcome),
            None if !still_running && before.is_some() && self.flushed == before => {
                if let Some(id) = before
                    && let Err(e) = store.delete_session(&id)
                {
                    outcome.error = Some(format!("kayıt silinemedi: {e}"));
                }
            }
            None => {}
        }
        if !still_running {
            self.flushed = None;
        }
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
    struct Script(VecDeque<Option<ActiveWindow>>, VecDeque<u64>);

    impl Script {
        fn windows(w: impl IntoIterator<Item = Option<ActiveWindow>>) -> Self {
            Script(w.into_iter().collect(), VecDeque::new())
        }
    }

    impl ActivityProvider for Script {
        type Error = Never;
        fn active_window(&mut self) -> Result<Option<ActiveWindow>, Never> {
            Ok(self.0.pop_front().flatten())
        }
        fn idle_seconds(&mut self) -> Result<u64, Never> {
            Ok(self.1.pop_front().unwrap_or(0))
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
        let mut tracker = Tracker::new(Script::windows(script), EngineConfig::default(), privacy);
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

    #[test]
    fn dropped_session_that_was_flushed_is_deleted() {
        let store = Store::open_in_memory().unwrap();
        // Pencereye geçip hiç dokunmadan 3 dk bekleme: oturum yazılır ama
        // boşluk başlangıcı oturum başına düştüğü için sonunda atılır.
        let n = 181;
        let windows = (0..n).map(|_| win("A"));
        let idles = (0..n as u64).collect::<VecDeque<_>>();
        let mut tracker = Tracker::new(
            Script(windows.collect(), idles),
            EngineConfig::default(),
            PrivacySettings::default(),
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..n {
            tracker.tick(&store, t0 + Duration::seconds(i as i64));
        }
        assert!(tracker.current().is_none());
        let apps = store.app_totals(t0, t0 + Duration::hours(1)).unwrap();
        assert!(apps.is_empty(), "boşta geçen süre sayıldı: {apps:?}");
    }
}
