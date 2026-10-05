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
    /// Monotonik saatin başlangıcı (bkz. [`Tracker::uptime`]).
    started: std::time::Instant,
}

impl<P: ActivityProvider> Tracker<P> {
    /// Boşta kaydı ayarı `config` yerine gizlilik ayarlarından gelir.
    pub fn new(provider: P, config: EngineConfig, privacy: PrivacySettings) -> Self {
        let config = EngineConfig {
            max_away: privacy.max_away(),
            ..config
        };
        Self {
            provider,
            engine: Engine::new(config),
            privacy,
            ticks: 0,
            flushed: None,
            started: std::time::Instant::now(),
        }
    }

    /// Makine uyurken ilerlemeyen saat: gözlem arasındaki uzun boşluğun uyku mu, gecikme mi
    /// olduğunu ayırır ([`Engine::tick_with_uptime`]). macOS'ta (ve Linux'ta) `Instant` uykuda
    /// durur; Windows'ta (QPC) uykuda da ilerlediği için ayırt edemez, kullanılmaz.
    fn uptime(&self) -> Option<std::time::Duration> {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            Some(self.started.elapsed())
        } else {
            None
        }
    }

    pub fn privacy(&self) -> &PrivacySettings {
        &self.privacy
    }

    /// Yeni ayarlar bir sonraki gözlemden itibaren geçerli olur.
    pub fn set_privacy(&mut self, privacy: PrivacySettings) {
        self.engine.set_max_away(privacy.max_away());
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
        // Duraklatılmışken pencere ve başlık hiç okunmaz (sonuç zaten atılırdı): hem
        // gizlilik beklentisi hem de saniyelik Erişilebilirlik çağrılarının maliyeti.
        if self.privacy.paused {
            return Observation {
                window: None,
                idle: 0,
                error: None,
            };
        }
        let privacy = &self.privacy;
        let read_title = |app_id: &str| privacy.reads_title(app_id);
        let (window, error) = match self.provider.active_window_with(&read_title) {
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
        // Takip başlamadan önceki süre (önceki çalışma kapanırken kaydedildi) boşta sayılmaz.
        if self.ticks == 0 {
            self.engine.forget_away(now);
        }

        let uptime = self.uptime();
        let closed = self
            .engine
            .tick_with_uptime(now, uptime, obs.window, obs.idle);
        self.settle(store, before, closed, &mut outcome);
        outcome.changed = self.engine.current().map(|s| s.id) != before;
        if let Some(away) = self.engine.take_away() {
            save(store, &away, &mut outcome);
            outcome.changed = true;
        }
        // Duraklatılmışken geçen süre boşta sayılmaz.
        if self.privacy.paused {
            self.engine.forget_away(now);
        }

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
        let uptime = self.uptime();
        let closed = self.engine.flush_with_uptime(now, uptime);
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

    #[test]
    fn clock_set_backwards_does_not_delete_the_saved_session() {
        let store = Store::open_in_memory().unwrap();
        let mut tracker = Tracker::new(
            Script::windows((0..12).map(|_| win("A"))),
            EngineConfig::default(),
            PrivacySettings::default(),
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..10 {
            tracker.tick(&store, t0 + Duration::seconds(i));
        }
        // Saat bir saat geri alındı; hemen ardından uygulama kapandı.
        let back = t0 - Duration::hours(1);
        tracker.tick(&store, back);
        assert!(
            tracker
                .shutdown(&store, back + Duration::seconds(1))
                .is_none()
        );
        let sessions = store
            .sessions_between(t0 - Duration::hours(2), t0 + Duration::hours(1))
            .unwrap();
        assert!(
            sessions
                .iter()
                .any(|s| s.started_at == t0 && s.duration() == Duration::seconds(9)),
            "{sessions:?}"
        );
    }

    #[test]
    fn idle_time_is_saved_as_away_but_not_counted_as_work() {
        let store = Store::open_in_memory().unwrap();
        // Son girdi 9. saniyede, 610. saniyede dönüş.
        let n = 10 + 600 + 5;
        let windows = (0..n).map(|_| win("A"));
        let idles: VecDeque<u64> = (0..n as u64)
            .map(|i| if (10..610).contains(&i) { i - 9 } else { 0 })
            .collect();
        let mut tracker = Tracker::new(
            Script(windows.collect(), idles),
            EngineConfig::default(),
            PrivacySettings::default(),
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..n {
            tracker.tick(&store, t0 + Duration::seconds(i as i64));
        }
        let all = store.sessions_between(t0, t0 + Duration::hours(1)).unwrap();
        let away: Vec<_> = all.iter().filter(|s| s.is_idle()).collect();
        assert_eq!(away.len(), 1);
        assert_eq!(away[0].duration(), Duration::seconds(601));
        // Toplamlar yalnızca çalışmayı sayar.
        let apps = store.app_totals(t0, t0 + Duration::hours(1)).unwrap();
        assert!(
            apps.iter().all(|u| u.key != crate::model::IDLE_APP_ID),
            "{apps:?}"
        );
    }

    #[test]
    fn away_does_not_reach_back_before_tracking_started() {
        let store = Store::open_in_memory().unwrap();
        // Açılışta kullanıcı 10 dakikadır boşta; 5 dakika sonra döner.
        let windows = (0..301).map(|_| win("A"));
        let idles: VecDeque<u64> = (0..301u64)
            .map(|i| if i < 300 { 600 + i } else { 0 })
            .collect();
        let mut tracker = Tracker::new(
            Script(windows.collect(), idles),
            EngineConfig::default(),
            PrivacySettings::default(),
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..301 {
            tracker.tick(&store, t0 + Duration::seconds(i));
        }
        let away: Vec<_> = store
            .sessions_between(t0 - Duration::hours(1), t0 + Duration::hours(1))
            .unwrap()
            .into_iter()
            .filter(|s| s.is_idle())
            .collect();
        assert_eq!(away.len(), 1);
        assert_eq!(away[0].started_at, t0);
    }

    #[test]
    fn paused_tracker_does_not_read_windows() {
        let store = Store::open_in_memory().unwrap();
        let privacy = PrivacySettings {
            paused: true,
            ..Default::default()
        };
        let mut tracker = Tracker::new(
            Script::windows([win("A"), win("A")]),
            EngineConfig::default(),
            privacy,
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..2 {
            tracker.tick(&store, t0 + Duration::seconds(i));
        }
        assert!(tracker.current().is_none());
        // Gözlemler tüketilmedi: sağlayıcıya hiç sorulmadı.
        assert_eq!(tracker.provider.0.len(), 2);
    }

    /// Başlığı yalnızca izin verildiğinde dolduran sağlayıcı; kararları kaydeder.
    struct TitleProbe(Vec<(String, bool)>);

    impl ActivityProvider for TitleProbe {
        type Error = Never;
        fn active_window(&mut self) -> Result<Option<ActiveWindow>, Never> {
            unreachable!("takipçi başlık iznini sormalı")
        }
        fn active_window_with(
            &mut self,
            read_title: &dyn Fn(&str) -> bool,
        ) -> Result<Option<ActiveWindow>, Never> {
            let app = ["com.test.Pass", "com.test.Mail", "com.test.Code"][self.0.len() % 3];
            let read = read_title(app);
            self.0.push((app.to_string(), read));
            Ok(Some(ActiveWindow {
                app_id: app.into(),
                app_name: app.into(),
                title: if read {
                    "gizli değil".into()
                } else {
                    String::new()
                },
                url: None,
            }))
        }
        fn idle_seconds(&mut self) -> Result<u64, Never> {
            Ok(0)
        }
    }

    #[test]
    fn titles_of_excluded_and_hidden_apps_are_never_read() {
        let store = Store::open_in_memory().unwrap();
        let privacy = PrivacySettings {
            excluded_apps: vec!["com.test.Pass".into()],
            hidden_title_apps: vec!["COM.TEST.MAIL".into()],
            ..Default::default()
        };
        let mut tracker = Tracker::new(TitleProbe(Vec::new()), EngineConfig::default(), privacy);
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..3 {
            tracker.tick(&store, t0 + Duration::seconds(i));
        }
        let reads: Vec<bool> = tracker.provider.0.iter().map(|(_, r)| *r).collect();
        assert_eq!(reads, [false, false, true]);
    }
}
