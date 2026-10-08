use chrono::{DateTime, Utc};

use uuid::Uuid;

use crate::calls::CallRecorder;
use crate::engine::{Clocks, Engine, EngineConfig, elapsed_clock};
use crate::model::{ActiveWindow, Session};
use crate::platform::{ActivityProvider, CallApps};
use crate::privacy::PrivacySettings;
use crate::store::Store;

/// Devam eden oturum bu kadar gözlemde bir diske yazılır (çökmede en fazla bu kadar saniye kaybolur).
const FLUSH_EVERY: u32 = 5;
/// Görüşme bu kadar gözlemde bir yoklanır (~5 sn).
const CALL_CHECK_EVERY: u32 = 5;
/// Ekranı uyanık tutan uygulama ancak bu kadar girdisiz süreden sonra sorulur (boşta eşiğinden
/// kısa; her saniye sorulmasın).
const WATCH_CHECK_AFTER_SECS: u64 = 60;
/// Girdisiz izleme en çok bu kadar çalışma sayılır; ekranı açık unutulan video geceyi doldurmasın.
const WATCH_MAX_SECS: u64 = 3 * 60 * 60;

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
    /// Görüşme sinyalleri; bu gözlemde yoklanmadıysa `None`.
    calls: Option<CallApps>,
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
    watching: Watching,
    calls: CallRecorder,
}

/// Girdisiz izleme: ekran bir video ya da görüşme için uyanık tutulurken geçen süre etkinlik
/// sayılır. Son girdiden beri izleme sürdükçe boşta süresi sıfırdır; izleme bitince boşta süresi
/// izlemenin bittiği andan sayılır (bitişe kadarki süre geriye dönük boşta olmaz).
#[derive(Debug, Default)]
struct Watching {
    /// İzlemenin son görüldüğü gözlemdeki ham boşta süresi (son girdiden beri, saniye).
    seen_at_idle: Option<u64>,
}

impl Watching {
    /// Ham boşta süresinden (`raw`) motora verilecek boşta süresi.
    fn idle(&mut self, raw: u64, kept_awake: bool) -> u64 {
        // Boşta süresi geri gitti: arada girdi oldu, eski izleme bu boşluğa ait değil.
        if self.seen_at_idle.is_some_and(|seen| raw < seen) {
            self.seen_at_idle = None;
        }
        if kept_awake && raw <= WATCH_MAX_SECS {
            self.seen_at_idle = Some(raw);
        }
        raw - self.seen_at_idle.unwrap_or(0)
    }
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
            watching: Watching::default(),
            calls: CallRecorder::default(),
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

    /// Saatler ([`Clocks`]): makine uyurken duran ve uykuda da ilerleyen.
    fn clocks(&self) -> Clocks {
        Clocks {
            uptime: self.uptime(),
            elapsed: elapsed_clock(self.started),
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
                calls: None,
            };
        }
        let privacy = &self.privacy;
        let read_title = |app_id: &str| privacy.reads_title(app_id);
        let (window, error) = match self.provider.active_window_with(&read_title) {
            Ok(w) => (w.and_then(|w| self.privacy.apply(w)), None),
            Err(e) => (None, Some(format!("pencere okunamadı: {e}"))),
        };
        let raw = self.provider.idle_seconds().unwrap_or(0);
        let kept_awake = self.privacy.count_watching
            && raw >= WATCH_CHECK_AFTER_SECS
            && self.provider.display_kept_awake();
        // Gözlemler saniyede bir: görüşme birkaç gözlemde bir yoklanır.
        let calls = (self.privacy.detect_calls && self.ticks.is_multiple_of(CALL_CHECK_EVERY))
            .then(|| self.provider.call_apps());
        Observation {
            window,
            idle: self.watching.idle(raw, kept_awake),
            error,
            calls,
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

        let closed = self
            .engine
            .tick_with_clocks(now, self.clocks(), obs.window, obs.idle);
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
        self.record_calls(store, now, obs.calls, &mut outcome);

        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks.is_multiple_of(FLUSH_EVERY)
            && let Some(current) = self.engine.current()
        {
            save(store, current, &mut outcome);
            self.flushed = Some(current.id);
        }
        outcome
    }

    /// Görüşme yoklamasını işler; duraklatılınca ya da görüşme kaydı kapatılınca süren görüşme
    /// son görüldüğü anda biter.
    fn record_calls(
        &mut self,
        store: &Store,
        now: DateTime<Utc>,
        signals: Option<CallApps>,
        outcome: &mut TickOutcome,
    ) {
        let calls = match signals {
            Some(signals) => {
                let config = store.timesheet_config().unwrap_or_default();
                self.calls
                    .observe(now, &crate::calls::in_call(&signals, &config))
            }
            None if self.privacy.paused || !self.privacy.detect_calls => {
                self.calls.finish().into_iter().collect()
            }
            None => Vec::new(),
        };
        for call in calls {
            if let Err(e) = store.upsert_call(&call) {
                outcome.error = Some(format!("görüşme yazılamadı: {e}"));
            }
        }
    }

    /// Kapanışta devam eden oturumu kaydeder.
    pub fn shutdown(&mut self, store: &Store, now: DateTime<Utc>) -> Option<String> {
        let mut outcome = TickOutcome::default();
        if let Some(call) = self.calls.finish()
            && let Err(e) = store.upsert_call(&call)
        {
            outcome.error = Some(format!("görüşme yazılamadı: {e}"));
        }
        let before = self.engine.current().map(|s| s.id);
        let closed = self.engine.flush_with_clocks(now, self.clocks());
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

    /// `Script`, ekranı uyanık tutan uygulamayla: son okunan boşta süresine göre (her gözlemde
    /// sorulmadığı için sırayla değil).
    struct Awake(Script, u64, fn(u64) -> bool);

    impl ActivityProvider for Awake {
        type Error = Never;
        fn active_window(&mut self) -> Result<Option<ActiveWindow>, Never> {
            self.0.active_window()
        }
        fn idle_seconds(&mut self) -> Result<u64, Never> {
            self.1 = self.0.idle_seconds()?;
            Ok(self.1)
        }
        fn display_kept_awake(&mut self) -> bool {
            (self.2)(self.1)
        }
    }

    /// Son girdi 9. saniyede; video 10–1000. saniyeler arasında oynar; 1300. saniyede dönüş.
    /// (Çalışma süresi, boşta süreleri.)
    fn watch_video(count_watching: bool) -> (Duration, Vec<Duration>) {
        let store = Store::open_in_memory().unwrap();
        let n = 1305;
        let windows = (0..n).map(|_| win("A"));
        let idles: VecDeque<u64> = (0..n as u64)
            .map(|i| if (10..1300).contains(&i) { i - 9 } else { 0 })
            .collect();
        let privacy = PrivacySettings {
            count_watching,
            ..PrivacySettings::default()
        };
        let mut tracker = Tracker::new(
            // Boşta süresi i - 9: video 10–1000. saniyelerde.
            Awake(Script(windows.collect(), idles), 0, |idle| {
                (1..=991).contains(&idle)
            }),
            EngineConfig::default(),
            privacy,
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..n {
            tracker.tick(&store, t0 + Duration::seconds(i as i64));
        }
        tracker.shutdown(&store, t0 + Duration::seconds(n as i64));
        let all = store.sessions_between(t0, t0 + Duration::hours(1)).unwrap();
        let (away, work): (Vec<_>, Vec<_>) = all.iter().partition(|s| s.is_idle());
        let first_work = work
            .iter()
            .map(|s| s.duration())
            .min_by_key(|d| -d.num_seconds());
        (
            first_work.unwrap_or_default(),
            away.iter().map(|s| s.duration()).collect(),
        )
    }

    #[test]
    fn watching_a_video_without_input_is_not_away() {
        let (work, away) = watch_video(true);
        // İzleme bitene kadar çalışma; boşta yalnızca video bittikten dönüşe kadar.
        assert!(
            (work - Duration::seconds(1000)).num_seconds().abs() <= 2,
            "{work:?}"
        );
        assert_eq!(away.len(), 1);
        assert!(
            (away[0] - Duration::seconds(300)).num_seconds().abs() <= 2,
            "{away:?}"
        );
    }

    #[test]
    fn watching_can_be_turned_off() {
        let (work, away) = watch_video(false);
        assert!(work <= Duration::seconds(10), "{work:?}");
        assert_eq!(away.len(), 1);
        assert!(
            (away[0] - Duration::seconds(1291)).num_seconds().abs() <= 2,
            "{away:?}"
        );
    }

    /// `Script`, görüşmeyle: `in_call(gözlem sırası)`.
    struct Calling(Script, u64, fn(u64) -> bool);

    impl ActivityProvider for Calling {
        type Error = Never;
        fn active_window(&mut self) -> Result<Option<ActiveWindow>, Never> {
            self.1 += 1;
            self.0.active_window()
        }
        fn idle_seconds(&mut self) -> Result<u64, Never> {
            self.0.idle_seconds()
        }
        fn call_apps(&mut self) -> CallApps {
            CallApps {
                microphone: if (self.2)(self.1) {
                    vec![
                        "com.microsoft.teams2".into(),
                        "com.globaldelight.Boom3D".into(),
                    ]
                } else {
                    vec!["com.globaldelight.Boom3D".into()]
                },
                display: Vec::new(),
            }
        }
    }

    fn run_call(detect_calls: bool) -> Vec<crate::calls::Call> {
        let store = Store::open_in_memory().unwrap();
        let n = 600;
        let privacy = PrivacySettings {
            detect_calls,
            ..PrivacySettings::default()
        };
        // Teams 100.–400. gözlemlerde mikrofonu kullanır; Boom 3D hep.
        let mut tracker = Tracker::new(
            Calling(Script::windows((0..n).map(|_| win("A"))), 0, |i| {
                (100..400).contains(&i)
            }),
            EngineConfig::default(),
            privacy,
        );
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        for i in 0..n {
            tracker.tick(&store, t0 + Duration::seconds(i as i64));
        }
        tracker.shutdown(&store, t0 + Duration::seconds(n as i64));
        store.calls_between(t0, t0 + Duration::hours(1)).unwrap()
    }

    #[test]
    fn calls_are_recorded_from_microphone_use_of_call_apps() {
        let calls = run_call(true);
        assert_eq!(calls.len(), 1, "{calls:?}");
        let c = &calls[0];
        assert_eq!(c.app_id, "com.microsoft.teams2");
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        // Yoklama 5 saniyede bir: uçlar en çok 5 sn kayar.
        assert!(
            (c.started_at - (t0 + Duration::seconds(99)))
                .num_seconds()
                .abs()
                <= 5
        );
        assert!(
            (c.ended_at - (t0 + Duration::seconds(399)))
                .num_seconds()
                .abs()
                <= 5
        );
        assert!(run_call(false).is_empty());
    }

    #[test]
    fn watching_counts_from_the_last_input_and_only_up_to_the_limit() {
        let mut w = Watching::default();
        assert_eq!(w.idle(100, false), 100);
        assert_eq!(w.idle(200, true), 0);
        // Video bitti: boşta süresi bitişten sayılır.
        assert_eq!(w.idle(260, false), 60);
        // Girdi oldu: eski izleme unutulur.
        assert_eq!(w.idle(5, false), 5);
        // Sınırdan sonra uyanık ekran sayılmaz; sınırdan sonrası boşta.
        assert_eq!(w.idle(WATCH_MAX_SECS, true), 0);
        assert_eq!(w.idle(WATCH_MAX_SECS + 600, true), 600);
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
