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
    /// Bilgisayardan uzakta geçen süre (boşta ya da uykuda) bundan uzun değilse "Boşta"
    /// kaydı olur; daha uzunu (gece gibi) kaydedilmez. `None`: boşta süre hiç kaydedilmez.
    pub max_away: Option<Duration>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            idle_threshold: Duration::minutes(3),
            max_gap: Duration::seconds(15),
            min_session: Duration::seconds(1),
            max_away: None,
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
    /// Kullanıcının uzaklaştığı an (son girdi ya da uykudan önce son görülen an).
    away_since: Option<DateTime<Utc>>,
    /// Kullanıcı dönünce kapanan, henüz alınmamış boşta kaydı.
    away: Option<Session>,
    /// Boşluk bundan önce başlamış sayılmaz (duraklatma bitince süren boşluk duraklatılan
    /// süreye taşmasın).
    away_floor: Option<DateTime<Utc>>,
    /// Son gözlemin anı: uyku, açık oturum olmasa da (hariç tutulan uygulama, pencere yok)
    /// iki gözlem arasındaki boşluktan anlaşılır.
    last_tick: Option<DateTime<Utc>>,
    /// Son gözlemdeki monotonik saat (bkz. [`Engine::tick_with_uptime`]).
    last_uptime: Option<std::time::Duration>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            current: None,
            away_since: None,
            away: None,
            away_floor: None,
            last_tick: None,
            last_uptime: None,
        }
    }

    /// Boşta kaydının en uzun süresi (`None`: kaydetme). Süren bir boşluğa da uygulanır.
    pub fn set_max_away(&mut self, max: Option<Duration>) {
        self.config.max_away = max;
    }

    /// Kullanıcı dönünce kapanan boşta kaydını alır (her kayıt bir kez döner).
    pub fn take_away(&mut self) -> Option<Session> {
        self.away.take()
    }

    /// Süren boşluğu kayıt üretmeden unutur (örn. takip duraklatıldı); sonraki boşluk
    /// `now`'dan önce başlamış sayılmaz. Sonraki gözleme kadarki ara da uyku sayılmaz
    /// (duraklatılmışken uyuyan makine boşta kaydı üretmesin).
    pub fn forget_away(&mut self, now: DateTime<Utc>) {
        self.away_since = None;
        self.away = None;
        self.away_floor = Some(now);
        self.last_tick = None;
        self.last_uptime = None;
    }

    /// Kullanıcının `at` anından beri uzakta olduğunu not eder (daha önceki bir an varsa o kalır).
    fn mark_away(&mut self, at: DateTime<Utc>) {
        let at = self.away_floor.map_or(at, |floor| at.max(floor));
        self.away_since.get_or_insert(at);
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
        self.tick_with_uptime(now, None, window, idle_seconds)
    }

    /// [`Engine::tick`], monotonik saatle: `uptime` makine uyurken ilerlemeyen bir saattir
    /// (macOS'ta `Instant`; Windows'ta `Instant` uykuda da ilerlediği için `None` verilmeli).
    /// Duvar saati uzun bir ara gösterirken monotonik saat de aynı kadar ilerlediyse makine
    /// uyumamış, süreç gecikmiştir (App Nap, yavaş Erişilebilirlik çağrısı): oturum sürer.
    pub fn tick_with_uptime(
        &mut self,
        now: DateTime<Utc>,
        uptime: Option<std::time::Duration>,
        window: Option<ActiveWindow>,
        idle_seconds: u64,
    ) -> Option<Session> {
        let gap = self.gap(now, uptime);
        self.last_tick = Some(now);
        self.last_uptime = uptime;
        let closed = match gap {
            // Makine uyuduysa son görülen andan sonrası sayılmaz; kullanıcı o andan beri
            // uzakta. Açık oturum olmasa da (hariç tutulan uygulama, pencere yok) son
            // gözleme bakılır.
            Gap::Slept(last) => {
                self.mark_away(last);
                self.close_at(None)
            }
            // Saat geri alındı: oturum son görülen anda kapanır (kaydedilmiş süresi korunur),
            // yenisi `now`'dan başlar. Süren boşluk ileriki bir andan başlamış olabilir: unutulur.
            Gap::Backwards => {
                self.away_since = None;
                self.away_floor = Some(now);
                self.close_at(None)
            }
            Gap::None => None,
        };
        // Oturum yukarıda kapandıysa `observe` ikinci bir oturum kapatamaz.
        let switched = self.observe(now, window, idle_seconds);
        closed.or(switched)
    }

    /// Son gözlemden `now`'a kadarki aranın türü.
    fn gap(&self, now: DateTime<Utc>, uptime: Option<std::time::Duration>) -> Gap {
        let Some(last) = self.last_tick else {
            return Gap::None;
        };
        if now < last {
            return Gap::Backwards;
        }
        let wall = now - last;
        if wall <= self.config.max_gap {
            return Gap::None;
        }
        // Monotonik saat aradaki sürenin neredeyse tamamında ilerlediyse uyku değil gecikme.
        // Temkinli: gecikme boşta eşiğinden uzunsa yine uyku sayılır.
        let awake = uptime
            .zip(self.last_uptime)
            .and_then(|(up, last_up)| up.checked_sub(last_up))
            .and_then(|d| Duration::from_std(d).ok());
        match awake {
            Some(awake)
                if wall - awake <= self.config.max_gap && awake <= self.config.idle_threshold =>
            {
                Gap::None
            }
            _ => Gap::Slept(last),
        }
    }

    /// Uygulama kapanırken devam eden oturumu kapatır.
    pub fn flush(&mut self, now: DateTime<Utc>) -> Option<Session> {
        self.flush_with_uptime(now, None)
    }

    /// [`Engine::flush`], monotonik saatle (bkz. [`Engine::tick_with_uptime`]). Kapanış
    /// uykudan hemen sonra (ya da saat geri alınınca) gelirse oturum son görülen anda kapanır;
    /// yoksa uyku çalışma sayılırdı.
    pub fn flush_with_uptime(
        &mut self,
        now: DateTime<Utc>,
        uptime: Option<std::time::Duration>,
    ) -> Option<Session> {
        let gap = self.gap(now, uptime);
        self.forget_away(now);
        match gap {
            Gap::None => self.close_at(Some(now)),
            Gap::Slept(_) | Gap::Backwards => self.close_at(None),
        }
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
            self.mark_away(now - idle);
            return self.close_at(Some(now - idle));
        }
        // Kullanıcı başında: uzaktaysa son girdiyle geri döndü.
        self.end_away(now - idle);
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

    /// Uzakta geçen süreyi `at` anında kapatır; eşikten kısa ya da `max_away`'den uzunsa atılır.
    fn end_away(&mut self, at: DateTime<Utc>) {
        let Some(since) = self.away_since.take() else {
            return;
        };
        let length = at - since;
        if let Some(max) = self.config.max_away
            && length >= self.config.idle_threshold
            && length <= max
        {
            self.away = Some(Session::idle(since, at));
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

/// İki gözlem arası.
enum Gap {
    /// Olağan ya da gecikmiş gözlem: oturum sürer.
    None,
    /// Makine uyudu; son görülen an.
    Slept(DateTime<Utc>),
    /// Sistem saati geri alındı.
    Backwards,
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

    fn up(secs: u64) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(secs))
    }

    #[test]
    fn stalled_ticks_extend_the_session_but_sleep_does_not() {
        // 40 sn gecikme (App Nap, yavaş çağrı): monotonik saat de 40 sn ilerledi.
        let mut e = Engine::new(EngineConfig::default());
        assert!(
            e.tick_with_uptime(t(0), up(100), win("Code", "x"), 0)
                .is_none()
        );
        assert!(
            e.tick_with_uptime(t(40), up(140), win("Code", "x"), 0)
                .is_none()
        );
        let cur = e.current().unwrap();
        assert_eq!((cur.started_at, cur.ended_at), (t(0), t(40)));

        // Uyku: duvar saati 1 saat, monotonik saat 1 sn ilerledi.
        let closed = e
            .tick_with_uptime(t(3640), up(141), win("Code", "x"), 0)
            .unwrap();
        assert_eq!((closed.started_at, closed.ended_at), (t(0), t(40)));
        assert_eq!(e.current().unwrap().started_at, t(3640));

        // Gecikme boşta eşiğinden uzunsa temkinle uyku sayılır.
        e.tick_with_uptime(t(3650), up(151), win("Code", "x"), 0);
        let closed = e
            .tick_with_uptime(t(3650 + 600), up(151 + 600), win("Code", "x"), 0)
            .unwrap();
        assert_eq!(closed.ended_at, t(3650));

        // Monotonik saat yoksa (Windows) eski davranış: uzun ara uykudur.
        let mut w = Engine::new(EngineConfig::default());
        w.tick_with_uptime(t(0), None, win("Code", "x"), 0);
        w.tick_with_uptime(t(5), None, win("Code", "x"), 0);
        assert!(
            w.tick_with_uptime(t(40), None, win("Code", "x"), 0)
                .is_some()
        );
    }

    #[test]
    fn shutdown_right_after_wake_does_not_count_the_sleep() {
        let mut e = Engine::new(EngineConfig::default());
        run(
            &mut e,
            &[(0, win("Code", "x"), 0), (10, win("Code", "x"), 0)],
        );
        let flushed = e.flush(t(10 + 3600)).unwrap();
        assert_eq!((flushed.started_at, flushed.ended_at), (t(0), t(10)));

        // Gecikmiş kapanış (monotonik saat de ilerledi) `now`'a kadar sayılır.
        let mut e = Engine::new(EngineConfig::default());
        e.tick_with_uptime(t(0), up(0), win("Code", "x"), 0);
        let flushed = e.flush_with_uptime(t(30), up(30)).unwrap();
        assert_eq!(flushed.ended_at, t(30));
    }

    #[test]
    fn clock_set_backwards_keeps_the_recorded_session() {
        let mut e = Engine::new(EngineConfig::default());
        run(
            &mut e,
            &[(100, win("Code", "x"), 0), (110, win("Code", "x"), 0)],
        );
        // Saat bir saat geri alındı: oturum atılmaz, son görülen anda kapanır.
        let closed = e.tick(t(110 - 3600), win("Code", "x"), 0).unwrap();
        assert_eq!((closed.started_at, closed.ended_at), (t(100), t(110)));
        let cur = e.current().unwrap();
        assert_eq!(cur.started_at, t(110 - 3600));
        // Geri alındıktan hemen sonra kapanış da önceki oturumu sıfırlamaz.
        let mut e2 = Engine::new(EngineConfig::default());
        run(
            &mut e2,
            &[(100, win("Code", "x"), 0), (110, win("Code", "x"), 0)],
        );
        let flushed = e2.flush(t(50)).unwrap();
        assert_eq!((flushed.started_at, flushed.ended_at), (t(100), t(110)));
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

    fn recording_away(max_minutes: i64) -> Engine {
        Engine::new(EngineConfig {
            max_away: Some(Duration::minutes(max_minutes)),
            ..Default::default()
        })
    }

    #[test]
    fn idle_time_becomes_an_away_session_when_the_user_returns() {
        let mut e = recording_away(180);
        let mut steps: Vec<_> = (0..=10).map(|s| (s, win("Code", "x"), 0)).collect();
        // 10. saniyeden sonra girdi yok; 600. saniyede kullanıcı döner.
        steps.extend((11..600).map(|s| (s, win("Code", "x"), (s - 10) as u64)));
        run(&mut e, &steps);
        assert!(e.take_away().is_none(), "kullanıcı dönmeden kayıt olmaz");

        e.tick(t(600), win("Code", "x"), 0);
        let away = e.take_away().expect("boşta kaydı");
        assert!(away.is_idle());
        assert_eq!((away.started_at, away.ended_at), (t(10), t(600)));
        assert!(e.take_away().is_none(), "bir kez döner");
    }

    #[test]
    fn sleep_becomes_away_but_short_and_overnight_gaps_do_not() {
        let mut e = recording_away(60);
        run(
            &mut e,
            &[(0, win("Code", "x"), 0), (5, win("Code", "x"), 0)],
        );
        // 20 dk uyku.
        e.tick(t(5 + 20 * 60), win("Code", "x"), 0);
        let away = e.take_away().expect("uyku boşta sayılır");
        assert_eq!((away.started_at, away.ended_at), (t(5), t(5 + 20 * 60)));

        // 1 dk uyku: eşiğin (3 dk) altında.
        let at = 5 + 20 * 60;
        e.tick(t(at + 60), win("Code", "x"), 0);
        assert!(e.take_away().is_none());

        // 10 saatlik gece: sınırdan (60 dk) uzun.
        e.tick(t(at + 61), win("Code", "x"), 0);
        e.tick(t(at + 61 + 10 * 3600), win("Code", "x"), 0);
        assert!(e.take_away().is_none());
    }

    #[test]
    fn sleep_without_an_open_session_becomes_away() {
        // Hariç tutulan uygulama ya da pencere yok: açık oturum olmadan kapak kapanır.
        let mut e = recording_away(60);
        run(&mut e, &[(0, None, 0), (5, None, 0)]);
        assert!(e.current().is_none());
        e.tick(t(5 + 20 * 60), win("Code", "x"), 0);
        let away = e.take_away().expect("uyku boşta sayılır");
        assert_eq!((away.started_at, away.ended_at), (t(5), t(5 + 20 * 60)));

        // Duraklatılmışken (her gözlemden sonra unutulur) uyku kayıt üretmez.
        let mut paused = recording_away(60);
        paused.tick(t(0), None, 0);
        paused.forget_away(t(0));
        paused.tick(t(20 * 60), win("Code", "x"), 0);
        assert!(paused.take_away().is_none());
    }

    #[test]
    fn away_is_not_recorded_when_disabled_or_forgotten() {
        let mut steps: Vec<_> = vec![(0, win("Code", "x"), 0)];
        steps.extend((1..400).map(|s| (s, win("Code", "x"), s as u64)));
        steps.push((400, win("Code", "x"), 0));

        let mut off = Engine::new(EngineConfig::default());
        run(&mut off, &steps);
        assert!(off.take_away().is_none());

        let mut forgot = recording_away(180);
        run(&mut forgot, &steps[..400]);
        forgot.forget_away(t(399));
        run(&mut forgot, &steps[400..]);
        assert!(forgot.take_away().is_none());
    }

    #[test]
    fn away_after_a_pause_starts_when_the_pause_ended() {
        // Duraklatma 0–300 sn; kullanıcı 100. saniyeden beri girdi yapmadı, 900'de döner.
        let mut e = recording_away(180);
        for s in 0..300 {
            e.forget_away(t(s));
        }
        run(
            &mut e,
            &[(300, win("Code", "x"), 200), (900, win("Code", "x"), 0)],
        );
        let away = e.take_away().expect("boşta kaydı");
        assert_eq!((away.started_at, away.ended_at), (t(299), t(900)));
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
