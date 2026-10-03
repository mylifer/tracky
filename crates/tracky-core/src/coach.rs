//! Hedefler ve hatırlatıcılar: kesintisiz çalışma süresini izleyip mola önerir,
//! günlük hedefe ulaşılınca haber verir. Saf mantık; bildirimi çağıran gösterir.

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
}

impl Default for Goals {
    fn default() -> Self {
        Self {
            daily_hours: 8.0,
            notify_goal: true,
            break_after_minutes: Some(60),
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
}

#[derive(Debug, Default)]
pub struct Coach {
    /// Kesintisiz çalışmanın başladığı an.
    streak_start: Option<DateTime<Utc>>,
    last_active: Option<DateTime<Utc>>,
    /// Son mola hatırlatması (aynı çalışmada tekrar için).
    last_break_nudge: Option<DateTime<Utc>>,
    goal_notified_on: Option<NaiveDate>,
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
}
