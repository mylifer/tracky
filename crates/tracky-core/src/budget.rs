//! Sözleşme bütçeleri: proje (ve istenirse müşteri) başına anlaşılan adam-gün ile o işe bugüne
//! kadar yazılan süre. Adam-gün, zaman çizelgesindeki gün saatiyle (varsayılan 8 sa) saate
//! çevrilir. Saf mantık; süreleri depo, bildirimi çağıran hazırlar.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Zaman çizelgesinde gün saati yoksa ya da geçersizse bir adam-gün bu kadar saattir.
pub const DEFAULT_DAY_HOURS: f64 = 8.0;

/// Bütçenin bu oranına gelince önceden uyarılır.
pub const BUDGET_WARN_RATIO: f64 = 0.8;

/// Bir adam-günün saniyesi; geçersiz gün saatinde varsayılan.
pub fn day_seconds(day_hours: f64) -> f64 {
    let hours = if day_hours.is_finite() && day_hours > 0.0 && day_hours <= 24.0 {
        day_hours
    } else {
        DEFAULT_DAY_HOURS
    };
    hours * 3600.0
}

/// Bir proje ya da müşterinin bütçesi ve harcanan süre.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetUsage {
    pub id: String,
    pub budget_days: f64,
    pub budget_seconds: i64,
    /// Bugüne kadar yazılan süre (müşteride projelerinin toplamı).
    pub used_seconds: i64,
}

impl BudgetUsage {
    /// Harcanan / bütçe (1 = doldu).
    pub fn ratio(&self) -> f64 {
        if self.budget_seconds <= 0 {
            return 0.0;
        }
        self.used_seconds as f64 / self.budget_seconds as f64
    }

    pub fn level(&self) -> Option<BudgetLevel> {
        if self.budget_seconds <= 0 {
            None
        } else if self.used_seconds >= self.budget_seconds {
            Some(BudgetLevel::Reached)
        } else if self.ratio() >= BUDGET_WARN_RATIO {
            Some(BudgetLevel::Near)
        } else {
            None
        }
    }
}

/// Tüm bütçeler; `day_hours` hesapta kullanılan gün saati.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    pub day_hours: f64,
    pub projects: Vec<BudgetUsage>,
    pub clients: Vec<BudgetUsage>,
}

/// Bütçeleri harcanan süreyle birleştirir. `projects`/`clients` (kimlik, adam-gün); `used`
/// proje başına bugüne kadarki süre (saniye); müşterinin harcaması bağlı projelerinin toplamıdır.
pub fn budgets(
    day_hours: f64,
    projects: &[(String, f64)],
    clients: &[(String, f64)],
    project_clients: &HashMap<String, String>,
    used: &HashMap<String, i64>,
) -> Budgets {
    let day = day_seconds(day_hours);
    let usage = |id: &String, days: f64, used: i64| BudgetUsage {
        id: id.clone(),
        budget_days: days,
        budget_seconds: (days * day).round() as i64,
        used_seconds: used,
    };
    Budgets {
        day_hours: day / 3600.0,
        projects: projects
            .iter()
            .map(|(id, days)| usage(id, *days, used.get(id).copied().unwrap_or(0)))
            .collect(),
        clients: clients
            .iter()
            .map(|(id, days)| {
                let spent = project_clients
                    .iter()
                    .filter(|(_, c)| *c == id)
                    .map(|(p, _)| used.get(p).copied().unwrap_or(0))
                    .sum();
                usage(id, *days, spent)
            })
            .collect(),
    }
}

/// Bildirilen eşik: %80 ve dolma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetLevel {
    Near,
    Reached,
}

/// Gösterilecek bütçe bildirimi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAlert {
    /// Müşteri bütçesi mi (değilse proje)?
    pub client: bool,
    pub id: String,
    pub level: BudgetLevel,
    pub used_seconds: i64,
    pub budget_seconds: i64,
}

/// Yeni geçilen eşikler. Her eşik bir kez bildirilir: `notified` (anahtar → bildirilen en yüksek
/// seviye) çağıran tarafından kalıcı saklanır, böylece uygulama yeniden açılınca tekrarlanmaz.
/// Bütçe artırılıp oran eşiğin altına düşerse kayıt da düşer; eşik yeniden geçilince yine
/// bildirilir. Doğrudan dolan bütçe için yalnızca "doldu" gelir. `skip`: bildirilmeyen projeler
/// (arşivdekiler); kayıtları korunur ki arşivden çıkınca eski eşikler tekrarlanmasın.
pub fn budget_alerts(
    budgets: &Budgets,
    skip: &HashSet<String>,
    notified: &mut BTreeMap<String, BudgetLevel>,
) -> Vec<BudgetAlert> {
    let rows = budgets
        .projects
        .iter()
        .map(|u| (false, u))
        .chain(budgets.clients.iter().map(|u| (true, u)));
    let mut alerts = Vec::new();
    let mut live = HashSet::new();
    for (client, u) in rows {
        let key = format!("{}:{}", if client { "client" } else { "project" }, u.id);
        live.insert(key.clone());
        if !client && skip.contains(&u.id) {
            continue;
        }
        let level = u.level();
        let prev = notified.get(&key).copied();
        if level > prev {
            alerts.push(BudgetAlert {
                client,
                id: u.id.clone(),
                level: level.expect("None'dan büyük"),
                used_seconds: u.used_seconds,
                budget_seconds: u.budget_seconds,
            });
        }
        match level {
            Some(l) if level != prev => {
                notified.insert(key, l);
            }
            None => {
                notified.remove(&key);
            }
            _ => {}
        }
    }
    // Bütçesi kaldırılan ya da silinen kayıtlar unutulur.
    notified.retain(|k, _| live.contains(k));
    alerts
}

/// Adam-gün, Türkçe yazımla en çok bir ondalık: "12", "12,5".
pub fn format_days(days: f64) -> String {
    let s = format!("{:.1}", (days * 10.0).round() / 10.0);
    s.strip_suffix(".0").unwrap_or(&s).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 8 * 3600;

    fn usage(used: i64) -> Budgets {
        budgets(
            8.0,
            &[("p".into(), 10.0)],
            &[],
            &HashMap::new(),
            &HashMap::from([("p".to_string(), used)]),
        )
    }

    #[test]
    fn man_days_use_timesheet_day_hours_with_fallback() {
        let b = budgets(
            7.5,
            &[("p".into(), 2.0)],
            &[],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(b.projects[0].budget_seconds, 15 * 3600);
        assert_eq!(b.day_hours, 7.5);
        for bad in [0.0, -1.0, f64::NAN, 30.0] {
            let b = budgets(
                bad,
                &[("p".into(), 1.5)],
                &[],
                &HashMap::new(),
                &HashMap::new(),
            );
            assert_eq!(b.projects[0].budget_seconds, 12 * 3600);
            assert_eq!(b.day_hours, 8.0);
        }
    }

    #[test]
    fn client_usage_sums_its_projects() {
        let links = HashMap::from([
            ("a".to_string(), "togg".to_string()),
            ("b".to_string(), "togg".to_string()),
            ("c".to_string(), "baska".to_string()),
        ]);
        let used = HashMap::from([
            ("a".to_string(), 3 * DAY),
            ("b".to_string(), DAY),
            ("c".to_string(), 5 * DAY),
        ]);
        let b = budgets(
            8.0,
            &[("a".into(), 4.0)],
            &[("togg".into(), 5.0), ("bos".into(), 1.0)],
            &links,
            &used,
        );
        assert_eq!(b.projects[0].used_seconds, 3 * DAY);
        assert_eq!(b.projects[0].ratio(), 0.75);
        assert_eq!(b.clients[0].used_seconds, 4 * DAY);
        assert_eq!(b.clients[0].level(), Some(BudgetLevel::Near));
        assert_eq!(b.clients[1].used_seconds, 0);
        assert_eq!(b.clients[1].level(), None);
    }

    #[test]
    fn levels_at_80_and_100_percent() {
        assert_eq!(usage(8 * DAY - 1).projects[0].level(), None);
        assert_eq!(usage(8 * DAY).projects[0].level(), Some(BudgetLevel::Near));
        assert_eq!(
            usage(10 * DAY).projects[0].level(),
            Some(BudgetLevel::Reached)
        );
    }

    #[test]
    fn each_threshold_is_notified_once() {
        let mut notified = BTreeMap::new();
        let skip = HashSet::new();
        assert!(budget_alerts(&usage(7 * DAY), &skip, &mut notified).is_empty());
        let near = budget_alerts(&usage(8 * DAY), &skip, &mut notified);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].level, BudgetLevel::Near);
        assert!(!near[0].client);
        assert!(budget_alerts(&usage(9 * DAY), &skip, &mut notified).is_empty());
        let full = budget_alerts(&usage(10 * DAY), &skip, &mut notified);
        assert_eq!(full[0].level, BudgetLevel::Reached);
        assert!(budget_alerts(&usage(12 * DAY), &skip, &mut notified).is_empty());

        // Kalıcı kayıt (yeniden açılış) aynı eşikleri tekrarlamaz.
        let mut reopened: BTreeMap<String, BudgetLevel> =
            serde_json::from_str(&serde_json::to_string(&notified).unwrap()).unwrap();
        assert!(budget_alerts(&usage(12 * DAY), &skip, &mut reopened).is_empty());

        // Doğrudan dolan bütçe için yalnız "doldu".
        let mut fresh = BTreeMap::new();
        let alerts = budget_alerts(&usage(11 * DAY), &skip, &mut fresh);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].level, BudgetLevel::Reached);
    }

    #[test]
    fn raised_budget_rearms_and_archived_projects_stay_quiet() {
        let mut notified = BTreeMap::new();
        let skip = HashSet::new();
        budget_alerts(&usage(10 * DAY), &skip, &mut notified);
        // Bütçe 20 güne çıkarıldı: oran %50, kayıt düşer; yeniden %80'de bildirilir.
        let raised = |used| {
            budgets(
                8.0,
                &[("p".into(), 20.0)],
                &[],
                &HashMap::new(),
                &HashMap::from([("p".to_string(), used)]),
            )
        };
        assert!(budget_alerts(&raised(10 * DAY), &skip, &mut notified).is_empty());
        assert!(notified.is_empty());
        assert_eq!(
            budget_alerts(&raised(16 * DAY), &skip, &mut notified).len(),
            1
        );

        // Arşivdeki proje bildirilmez; kaydı da silinmez.
        let skip = HashSet::from(["p".to_string()]);
        let mut notified = BTreeMap::new();
        assert!(budget_alerts(&usage(10 * DAY), &skip, &mut notified).is_empty());
        let mut kept = BTreeMap::from([("project:p".to_string(), BudgetLevel::Near)]);
        budget_alerts(&usage(10 * DAY), &skip, &mut kept);
        assert_eq!(kept.len(), 1);

        // Bütçesi kaldırılan projenin kaydı unutulur.
        budget_alerts(&Budgets::default(), &HashSet::new(), &mut kept);
        assert!(kept.is_empty());
    }

    #[test]
    fn days_are_formatted_in_turkish() {
        assert_eq!(format_days(12.0), "12");
        assert_eq!(format_days(12.54), "12,5");
        assert_eq!(format_days(0.04), "0");
    }
}
