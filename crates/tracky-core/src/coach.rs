//! Hedefler ve hatırlatıcılar: kesintisiz çalışma süresini izleyip mola önerir,
//! günlük hedefe ulaşılınca haber verir, akşam günün özetini ister. Saf mantık;
//! bildirimi çağıran gösterir.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Kullanıcının hedef ve hatırlatıcı ayarları.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Goals {
    /// Günlük çalışma hedefi (saat).
    pub daily_hours: f64,
    /// Hedefe ulaşınca bildir.
    pub notify_goal: bool,
    /// Bu kadar dakika kesintisiz çalışınca mola hatırlat; `None` = kapalı.
    pub break_after_minutes: Option<u32>,
    /// Kategori başına günlük üst sınırlar.
    pub limits: Vec<CategoryLimit>,
    /// Gün sonu özeti saati (yerel gece yarısından dakika); `None` = kapalı.
    pub day_summary_at: Option<u32>,
    /// Yeni haftanın ilk çalışmasında geçen haftanın özeti.
    pub weekly_summary: bool,
    /// Odak zamanlayıcısı sürerken dikkat dağıtıcı kategoriye geçince uyar.
    pub focus_guard: bool,
    /// Odak korumasının dikkat dağıtıcı saydığı kategoriler.
    pub distracting: Vec<String>,
    /// Proje başına haftalık hedefler.
    pub project_goals: Vec<ProjectGoal>,
}

/// Bir kategoride günde en fazla `minutes` dakika.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryLimit {
    pub category_id: String,
    pub minutes: u32,
}

impl CategoryLimit {
    pub fn seconds(&self) -> i64 {
        i64::from(self.minutes) * 60
    }
}

/// Bir projede haftada hedeflenen `minutes` dakika.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGoal {
    pub project_id: String,
    pub minutes: u32,
}

impl ProjectGoal {
    pub fn seconds(&self) -> i64 {
        i64::from(self.minutes) * 60
    }
}

/// Bundan az çalışılan günde özet gösterilmez.
pub const DAY_SUMMARY_MIN_SECS: i64 = 15 * 60;

/// Odak korumasının iki uyarısı arasındaki en kısa süre.
pub const DISTRACTION_COOLDOWN: Duration = Duration::minutes(3);

/// Limitin bu oranına gelince önceden uyarılır.
pub const LIMIT_WARN_RATIO: f64 = 0.8;

impl Default for Goals {
    fn default() -> Self {
        Self {
            daily_hours: 8.0,
            notify_goal: true,
            break_after_minutes: Some(60),
            limits: Vec::new(),
            day_summary_at: Some(18 * 60),
            weekly_summary: true,
            focus_guard: true,
            distracting: vec![crate::classify::default_category_id("Sosyal & Eğlence")],
            project_goals: Vec::new(),
        }
    }
}

impl Goals {
    pub fn daily_seconds(&self) -> i64 {
        (self.daily_hours.clamp(0.0, 24.0) * 3600.0).round() as i64
    }
}

/// Bu kadar etkinliksiz geçen süre mola sayılır ve kesintisiz çalışmayı sıfırlar.
pub const BREAK_RESETS_AFTER: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nudge {
    /// `minutes` dakikadır kesintisiz çalışılıyor.
    TakeBreak { minutes: i64 },
    /// Günlük hedef (`seconds`) aşıldı.
    GoalReached { seconds: i64 },
    /// Kategori limitinin %80'ine gelindi.
    LimitNear {
        category_id: String,
        limit: i64,
        used: i64,
    },
    /// Kategori limiti aşıldı.
    LimitReached { category_id: String, limit: i64 },
    /// Gün sonu özeti zamanı (içeriği çağıran rapordan hazırlar).
    DaySummary,
    /// Odak sırasında dikkat dağıtıcı uygulamaya geçildi.
    Distraction { app_name: String, minutes_left: i64 },
    /// Projenin haftalık hedefi (`target` saniye) doldu.
    ProjectGoalReached { project_id: String, target: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LimitLevel {
    Near,
    Reached,
}

#[derive(Debug, Default)]
pub struct Coach {
    /// Kesintisiz çalışmanın başladığı an.
    streak_start: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    /// Son mola hatırlatması (aynı çalışmada tekrar için).
    last_break_nudge: Option<DateTime<Utc>>,
    goal_notified_on: Option<NaiveDate>,
    /// Bugün gösterilen limit uyarıları: (kategori, seviye).
    limits_notified: HashSet<(String, LimitLevel)>,
    limits_day: Option<NaiveDate>,
    /// Bu hafta hedefi dolduğu bildirilen projeler.
    goals_notified: HashSet<String>,
    goals_week: Option<NaiveDate>,
    summary_sent_on: Option<NaiveDate>,
    last_distraction: Option<DateTime<Utc>>,
}

impl Coach {
    pub fn new() -> Self {
        Self::default()
    }

    /// Saniyede bir çağrılır. `active` = şu an kaydedilen bir etkinlik var;
    /// `today` yerel gün, `today_seconds` o günün toplam süresi.
    pub fn observe(
        &mut self,
        goals: &Goals,
        now: DateTime<Utc>,
        active: bool,
        today: NaiveDate,
        today_seconds: i64,
    ) -> Vec<Nudge> {
        let mut nudges = Vec::new();

        if active {
            let resumed = self
                .last_active
                .is_none_or(|last| now - last >= BREAK_RESETS_AFTER);
            if resumed {
                self.streak_start = Some(now);
                self.last_break_nudge = None;
            }
            self.last_active = Some(now);
        } else if self
            .last_active
            .is_some_and(|last| now - last >= BREAK_RESETS_AFTER)
        {
            self.streak_start = None;
            self.last_break_nudge = None;
        }

        if let (true, Some(minutes), Some(start)) =
            (active, goals.break_after_minutes, self.streak_start)
        {
            let every = Duration::minutes(i64::from(minutes.max(1)));
            let since = self.last_break_nudge.unwrap_or(start);
            if now - since >= every {
                self.last_break_nudge = Some(now);
                nudges.push(Nudge::TakeBreak {
                    minutes: (now - start).num_minutes(),
                });
            }
        }

        let target = goals.daily_seconds();
        if goals.notify_goal
            && target > 0
            && today_seconds >= target
            && self.goal_notified_on != Some(today)
        {
            self.goal_notified_on = Some(today);
            nudges.push(Nudge::GoalReached { seconds: target });
        }

        nudges
    }

    /// Kategori limitlerini denetler. `used` bugünkü kategori süreleri (saniye).
    /// Her limit için günde en çok bir "yaklaştın" ve bir "aştın" bildirimi.
    pub fn observe_limits(
        &mut self,
        goals: &Goals,
        today: NaiveDate,
        used: &HashMap<String, i64>,
    ) -> Vec<Nudge> {
        if self.limits_day != Some(today) {
            self.limits_day = Some(today);
            self.limits_notified.clear();
        }
        let mut nudges = Vec::new();
        for limit in &goals.limits {
            let (secs, spent) = (
                limit.seconds(),
                used.get(&limit.category_id).copied().unwrap_or(0),
            );
            if secs <= 0 {
                continue;
            }
            let key = |level| (limit.category_id.clone(), level);
            if spent >= secs {
                // Aşıldıysa ayrıca "yaklaştın" demeye gerek yok.
                self.limits_notified.insert(key(LimitLevel::Near));
                if self.limits_notified.insert(key(LimitLevel::Reached)) {
                    nudges.push(Nudge::LimitReached {
                        category_id: limit.category_id.clone(),
                        limit: secs,
                    });
                }
            } else if spent as f64 >= secs as f64 * LIMIT_WARN_RATIO
                && self.limits_notified.insert(key(LimitLevel::Near))
            {
                nudges.push(Nudge::LimitNear {
                    category_id: limit.category_id.clone(),
                    limit: secs,
                    used: spent,
                });
            }
        }
        nudges
    }

    /// Uygulama gün içinde yeniden açıldığında, zaten geçilmiş eşikler için
    /// bildirimi tekrarlamamak üzere bugünkü durumu sessizce işaretler.
    pub fn prime_limits(&mut self, goals: &Goals, today: NaiveDate, used: &HashMap<String, i64>) {
        self.observe_limits(goals, today, used);
    }

    /// Gün sonu özeti zamanı geldiyse günde bir kez `DaySummary`. `minute` yerel
    /// gece yarısından geçen dakika, `today_seconds` günün toplam süresi.
    pub fn observe_summary(
        &mut self,
        goals: &Goals,
        today: NaiveDate,
        minute: u32,
        today_seconds: i64,
    ) -> Option<Nudge> {
        let at = goals.day_summary_at?;
        if minute < at
            || today_seconds < DAY_SUMMARY_MIN_SECS
            || self.summary_sent_on == Some(today)
        {
            return None;
        }
        self.summary_sent_on = Some(today);
        Some(Nudge::DaySummary)
    }

    /// Proje hedeflerini denetler. `week` haftanın pazartesisi, `used` bu haftaki proje
    /// süreleri (saniye). Her proje için haftada en çok bir bildirim.
    pub fn observe_project_goals(
        &mut self,
        goals: &Goals,
        week: NaiveDate,
        used: &HashMap<String, i64>,
    ) -> Vec<Nudge> {
        if self.goals_week != Some(week) {
            self.goals_week = Some(week);
            self.goals_notified.clear();
        }
        goals
            .project_goals
            .iter()
            .filter(|g| g.minutes > 0)
            .filter(|g| used.get(&g.project_id).copied().unwrap_or(0) >= g.seconds())
            .filter(|g| self.goals_notified.insert(g.project_id.clone()))
            .map(|g| Nudge::ProjectGoalReached {
                project_id: g.project_id.clone(),
                target: g.seconds(),
            })
            .collect()
    }

    /// Hafta içinde yeniden açıldı: zaten dolmuş hedefleri sessizce işaretle.
    pub fn prime_project_goals(
        &mut self,
        goals: &Goals,
        week: NaiveDate,
        used: &HashMap<String, i64>,
    ) {
        self.observe_project_goals(goals, week, used);
    }

    /// Odak zamanlayıcısı sürerken (`focus_ends`) yeni bir pencereye geçildiğinde çağrılır;
    /// pencerenin kategorisi dikkat dağıtıcıysa ve son uyarıdan beri yeterince geçtiyse uyarır.
    pub fn observe_switch(
        &mut self,
        goals: &Goals,
        now: DateTime<Utc>,
        focus_ends: Option<DateTime<Utc>>,
        app_name: &str,
        category: Option<&str>,
    ) -> Option<Nudge> {
        let ends = focus_ends.filter(|e| *e > now)?;
        let distracting = category.is_some_and(|c| goals.distracting.iter().any(|d| d == c));
        if !goals.focus_guard
            || !distracting
            || self
                .last_distraction
                .is_some_and(|t| now - t < DISTRACTION_COOLDOWN)
        {
            return None;
        }
        self.last_distraction = Some(now);
        Some(Nudge::Distraction {
            app_name: app_name.to_string(),
            // Son dakikada "0 dk" yerine yukarı yuvarla.
            minutes_left: ((ends - now).num_seconds() + 59) / 60,
        })
    }

    /// Uygulama özet saatinden sonra açıldı: bugünün özetini gösterme
    /// (önceki oturumda gösterilmiş olabilir).
    pub fn mark_summary_sent(&mut self, today: NaiveDate) {
        self.summary_sent_on = Some(today);
    }

    /// Hedefe bugün zaten ulaşıldıysa (uygulama gün içinde yeniden açıldı)
    /// bildirimi tekrarlama.
    pub fn mark_goal_notified(&mut self, today: NaiveDate) {
        self.goal_notified_on = Some(today);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::minutes(min)
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 3).unwrap()
    }

    fn goals(break_after: u32) -> Goals {
        Goals {
            daily_hours: 8.0,
            notify_goal: true,
            break_after_minutes: Some(break_after),
            limits: Vec::new(),
            day_summary_at: Some(18 * 60),
            weekly_summary: true,
            ..Goals::default()
        }
    }

    /// `from..to` dakikaları arasında her dakika gözlem; çıkan hatırlatmalar.
    fn run(c: &mut Coach, g: &Goals, from: i64, to: i64, active: bool) -> Vec<(i64, Nudge)> {
        (from..to)
            .flat_map(|m| {
                c.observe(g, t(m), active, day(), 0)
                    .into_iter()
                    .map(move |n| (m, n))
            })
            .collect()
    }

    #[test]
    fn reminds_after_continuous_work_and_repeats() {
        let (mut c, g) = (Coach::new(), goals(50));
        let n = run(&mut c, &g, 0, 101, true);
        assert_eq!(
            n,
            [
                (50, Nudge::TakeBreak { minutes: 50 }),
                (100, Nudge::TakeBreak { minutes: 100 })
            ]
        );
    }

    #[test]
    fn short_pauses_do_not_reset_but_breaks_do() {
        let (mut c, g) = (Coach::new(), goals(50));
        assert!(run(&mut c, &g, 0, 30, true).is_empty());
        // 3 dakikalık duraklama: aynı çalışma sayılır.
        assert!(run(&mut c, &g, 30, 33, false).is_empty());
        assert_eq!(run(&mut c, &g, 33, 51, true).len(), 1);

        // 10 dakikalık mola sıfırlar.
        let (mut c, g) = (Coach::new(), goals(50));
        run(&mut c, &g, 0, 40, true);
        run(&mut c, &g, 40, 50, false);
        let n = run(&mut c, &g, 50, 101, true);
        assert_eq!(n, [(100, Nudge::TakeBreak { minutes: 50 })]);
    }

    #[test]
    fn disabled_break_reminder_stays_quiet() {
        let mut c = Coach::new();
        let g = Goals {
            break_after_minutes: None,
            ..goals(50)
        };
        assert!(run(&mut c, &g, 0, 300, true).is_empty());
    }

    #[test]
    fn goal_notifies_once_per_day() {
        let (mut c, g) = (Coach::new(), goals(500));
        let target = g.daily_seconds();
        assert!(c.observe(&g, t(0), true, day(), target - 1).is_empty());
        assert_eq!(
            c.observe(&g, t(1), true, day(), target),
            [Nudge::GoalReached { seconds: target }]
        );
        assert!(c.observe(&g, t(2), true, day(), target + 60).is_empty());
        let tomorrow = day().succ_opt().unwrap();
        assert_eq!(c.observe(&g, t(3), true, tomorrow, target).len(), 1);

        let mut c = Coach::new();
        c.mark_goal_notified(day());
        assert!(c.observe(&g, t(0), true, day(), target).is_empty());
    }

    #[test]
    fn limits_warn_once_then_notify_once_per_day() {
        let mut c = Coach::new();
        let g = Goals {
            limits: vec![CategoryLimit {
                category_id: "fun".into(),
                minutes: 60,
            }],
            ..goals(50)
        };
        let used = |secs: i64| HashMap::from([("fun".to_string(), secs)]);
        assert!(c.observe_limits(&g, day(), &used(47 * 60)).is_empty());
        assert_eq!(
            c.observe_limits(&g, day(), &used(48 * 60)),
            [Nudge::LimitNear {
                category_id: "fun".into(),
                limit: 3600,
                used: 48 * 60
            }]
        );
        assert!(c.observe_limits(&g, day(), &used(50 * 60)).is_empty());
        assert_eq!(
            c.observe_limits(&g, day(), &used(3600)),
            [Nudge::LimitReached {
                category_id: "fun".into(),
                limit: 3600
            }]
        );
        assert!(c.observe_limits(&g, day(), &used(4000)).is_empty());

        // Ertesi gün sıfırlanır; doğrudan aşılırsa yalnızca "aştın" gelir.
        let tomorrow = day().succ_opt().unwrap();
        assert_eq!(c.observe_limits(&g, tomorrow, &used(4000)).len(), 1);

        // Yeniden açılışta zaten geçilmiş eşikler tekrarlanmaz.
        let mut c = Coach::new();
        c.prime_limits(&g, day(), &used(3700));
        assert!(c.observe_limits(&g, day(), &used(3800)).is_empty());
    }

    #[test]
    fn day_summary_once_after_time_with_enough_work() {
        let (mut c, g) = (Coach::new(), goals(50));
        let min = DAY_SUMMARY_MIN_SECS;
        assert_eq!(c.observe_summary(&g, day(), 17 * 60 + 59, min), None);
        assert_eq!(c.observe_summary(&g, day(), 18 * 60, min - 1), None);
        assert_eq!(
            c.observe_summary(&g, day(), 18 * 60 + 5, min),
            Some(Nudge::DaySummary)
        );
        assert_eq!(c.observe_summary(&g, day(), 19 * 60, min), None);
        let tomorrow = day().succ_opt().unwrap();
        assert_eq!(
            c.observe_summary(&g, tomorrow, 18 * 60, min),
            Some(Nudge::DaySummary)
        );

        let off = Goals {
            day_summary_at: None,
            ..goals(50)
        };
        assert_eq!(
            Coach::new().observe_summary(&off, day(), 23 * 60, min * 10),
            None
        );

        let mut c = Coach::new();
        c.mark_summary_sent(day());
        assert_eq!(c.observe_summary(&g, day(), 20 * 60, min), None);
    }

    #[test]
    fn goals_saved_before_day_summary_get_the_default() {
        let g: Goals = serde_json::from_str(r#"{"dailyHours":6,"notifyGoal":false}"#).unwrap();
        assert_eq!(g.day_summary_at, Some(18 * 60));
        assert!(g.weekly_summary);
        assert_eq!(g.daily_hours, 6.0);
    }

    #[test]
    fn focus_guard_warns_on_distracting_switch_with_cooldown() {
        let mut c = Coach::new();
        let g = goals(50);
        let fun = crate::classify::default_category_id("Sosyal & Eğlence");
        let ends = Some(t(25));
        // Odak yokken ya da kategori dikkat dağıtıcı değilken sessiz.
        assert_eq!(
            c.observe_switch(&g, t(0), None, "YouTube", Some(&fun)),
            None
        );
        assert_eq!(c.observe_switch(&g, t(0), ends, "Code", Some("dev")), None);
        assert_eq!(c.observe_switch(&g, t(0), ends, "Bilinmeyen", None), None);
        assert_eq!(
            c.observe_switch(&g, t(1), ends, "YouTube", Some(&fun)),
            Some(Nudge::Distraction {
                app_name: "YouTube".into(),
                minutes_left: 24
            })
        );
        // 3 dakika dolmadan tekrar uyarmaz, sonra uyarır.
        assert_eq!(
            c.observe_switch(&g, t(3), ends, "Instagram", Some(&fun)),
            None
        );
        assert!(
            c.observe_switch(&g, t(4), ends, "Instagram", Some(&fun))
                .is_some()
        );
        // Süre dolduysa ya da kapalıysa sessiz.
        assert_eq!(
            c.observe_switch(&g, t(30), ends, "YouTube", Some(&fun)),
            None
        );
        let off = Goals {
            focus_guard: false,
            ..goals(50)
        };
        assert_eq!(
            Coach::new().observe_switch(&off, t(1), ends, "YouTube", Some(&fun)),
            None
        );
    }

    #[test]
    fn project_goals_notify_once_per_week() {
        let mut c = Coach::new();
        let g = Goals {
            project_goals: vec![ProjectGoal {
                project_id: "kum".into(),
                minutes: 120,
            }],
            ..goals(50)
        };
        let used = |secs: i64| HashMap::from([("kum".to_string(), secs)]);
        assert!(c.observe_project_goals(&g, day(), &used(7199)).is_empty());
        assert_eq!(
            c.observe_project_goals(&g, day(), &used(7200)),
            [Nudge::ProjectGoalReached {
                project_id: "kum".into(),
                target: 7200
            }]
        );
        assert!(c.observe_project_goals(&g, day(), &used(9000)).is_empty());
        let next_week = day() + Duration::days(7);
        assert_eq!(c.observe_project_goals(&g, next_week, &used(7200)).len(), 1);

        let mut c = Coach::new();
        c.prime_project_goals(&g, day(), &used(8000));
        assert!(c.observe_project_goals(&g, day(), &used(9000)).is_empty());
    }
}
