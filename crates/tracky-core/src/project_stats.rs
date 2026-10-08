//! Proje profili: bir projeye giden sürenin günlere, günün saatlerine, uygulamalara, pencere
//! başlıklarına ve kategorilere dağılımı; projenin çalışma bloklarından odak süresi.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::blocks::{self, Activity};
use crate::classify::Classifier;
use crate::model::Session;
use crate::report::Bucket;

/// Kırpılmış oturum dilimi: (başlangıç, bitiş, oturum, kategori, proje).
type Span<'a> = (
    DateTime<Utc>,
    DateTime<Utc>,
    &'a Session,
    Option<String>,
    Option<String>,
);

/// Listelenen en çok uygulama / başlık sayısı.
const TOP: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTotal {
    pub app_id: String,
    pub app_name: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleTotal {
    pub app_name: String,
    pub title: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub total_seconds: i64,
    /// Gün başına süre (saniye), `day_starts` sırasıyla.
    pub days: Vec<i64>,
    /// Yerel saate göre günün 24 saatine dağılım (saniye).
    pub hours: Vec<i64>,
    pub apps: Vec<AppTotal>,
    pub titles: Vec<TitleTotal>,
    /// Projenin süresinin kategorilere dağılımı (kategorisiz: `id: None`).
    pub categories: Vec<Bucket>,
    /// Çoğunluğu bu projede geçen çalışma blokları.
    pub blocks: u32,
    /// Bu blokların ortalama etkin süresi (saniye).
    pub focus_seconds: i64,
    /// Bu bloklarda etkin saat başına uygulama değişimi (×10).
    pub switches_per_hour_x10: u32,
}

/// `[from, to)` aralığında `project` projesinin profili. `day_starts` yerel gün sınırlarıdır
/// (artan sırada, ilk öğe `from`); `utc_offset` bir anın yerel saat farkı (saniye).
pub fn build(
    sessions: &[Session],
    classifier: &Classifier,
    project: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    day_starts: &[DateTime<Utc>],
    utc_offset: impl Fn(DateTime<Utc>) -> i32,
) -> ProjectStats {
    // Süreler milisaniye olarak toplanır, en sonda saniyeye çevrilir.
    let mut days = vec![0_i64; day_starts.len()];
    let mut hours = vec![0_i64; 24];
    let mut apps: HashMap<&str, (&str, i64)> = HashMap::new();
    let mut titles: HashMap<(&str, &str), i64> = HashMap::new();
    let mut categories: HashMap<Option<String>, i64> = HashMap::new();
    let mut total = 0;
    // Blok analizi tüm etkinliklerle yapılır (araya giren başka işler bloğu böler).
    let mut spans: Vec<Span> = Vec::new();

    for s in sessions {
        let (start, end) = (s.started_at.max(from), s.ended_at.min(to));
        if end <= start || !s.counts_as_work() {
            continue;
        }
        let class = classifier.classify(s);
        let mine = class.project.as_deref() == Some(project);
        spans.push((start, end, s, class.category.clone(), class.project));
        if !mine {
            continue;
        }
        let ms = (end - start).num_milliseconds();
        total += ms;
        *categories.entry(class.category).or_default() += ms;
        apps.entry(s.app_id.as_str())
            .or_insert((s.app_name.as_str(), 0))
            .1 += ms;
        let title = if s.title.trim().is_empty() {
            s.app_name.as_str()
        } else {
            s.title.as_str()
        };
        *titles.entry((s.app_name.as_str(), title)).or_default() += ms;
        for (i, day_start) in day_starts.iter().enumerate() {
            let day_end = day_starts.get(i + 1).copied().unwrap_or(to);
            let (a, b) = (start.max(*day_start), end.min(day_end));
            if b > a {
                days[i] += (b - a).num_milliseconds();
            }
        }
        // Saat sınırını aşan oturum her saate kendi payı kadar yazılır.
        let mut a = start;
        while a < end {
            let local = a.timestamp_millis() + i64::from(utc_offset(a)) * 1000;
            let hour = (local.rem_euclid(86_400_000) / 3_600_000) as usize;
            let b = (a + Duration::milliseconds(3_600_000 - local.rem_euclid(3_600_000))).min(end);
            hours[hour] += (b - a).num_milliseconds();
            a = b;
        }
    }

    let mut focus_ms = 0;
    let mut block_count = 0_u32;
    let mut switches = 0_u32;
    for (i, day_start) in day_starts.iter().enumerate() {
        let day_end = day_starts.get(i + 1).copied().unwrap_or(to);
        let items: Vec<Activity> = spans
            .iter()
            .filter_map(|(start, end, s, category, project)| {
                let (a, b) = ((*start).max(*day_start), (*end).min(day_end));
                (b > a).then(|| Activity {
                    start: a,
                    end: b,
                    app_id: &s.app_id,
                    app_name: &s.app_name,
                    category: category.as_deref(),
                    project: project.as_deref(),
                    block_start: Activity::starts_block(s, a),
                })
            })
            .collect();
        for b in blocks::analyze(&items).blocks {
            if b.project_id.as_deref() == Some(project) {
                block_count += 1;
                focus_ms += b.active_seconds * 1000;
                switches += b.switches;
            }
        }
    }

    let mut apps: Vec<AppTotal> = apps
        .into_iter()
        .map(|(app_id, (app_name, ms))| AppTotal {
            app_id: app_id.to_string(),
            app_name: app_name.to_string(),
            seconds: ms / 1000,
        })
        .filter(|a| a.seconds > 0)
        .collect();
    apps.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));
    apps.truncate(TOP);
    let mut titles: Vec<TitleTotal> = titles
        .into_iter()
        .map(|((app_name, title), ms)| TitleTotal {
            app_name: app_name.to_string(),
            title: title.to_string(),
            seconds: ms / 1000,
        })
        .filter(|t| t.seconds > 0)
        .collect();
    titles.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.title.cmp(&b.title)));
    titles.truncate(TOP);
    let mut categories: Vec<Bucket> = categories
        .into_iter()
        .map(|(id, ms)| Bucket {
            id,
            seconds: ms / 1000,
        })
        .filter(|b| b.seconds > 0)
        .collect();
    categories.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.id.cmp(&b.id)));

    ProjectStats {
        total_seconds: total / 1000,
        days: days.into_iter().map(|ms| ms / 1000).collect(),
        hours: hours.into_iter().map(|ms| ms / 1000).collect(),
        apps,
        titles,
        categories,
        blocks: block_count,
        focus_seconds: if block_count > 0 {
            focus_ms / 1000 / i64::from(block_count)
        } else {
            0
        },
        switches_per_hour_x10: if focus_ms > 0 {
            (i64::from(switches) * 36_000_000 / focus_ms) as u32
        } else {
            0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Rule, RuleField, Tag, TagKind};
    use chrono::TimeZone;
    use uuid::Uuid;

    /// 2026-09-21 00:00 UTC'den `m` dakika sonra.
    fn t(m: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 21, 0, 0, 0).unwrap() + Duration::minutes(m)
    }

    fn s(app: &str, title: &str, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
            block_from: None,
        }
    }

    fn classifier() -> Classifier {
        let tag = |id: &str, kind| Tag {
            id: id.into(),
            kind,
            name: id.into(),
            color: 1,
        };
        let rule = |tag: &str, field, pattern: &str| Rule {
            id: Uuid::new_v4().to_string(),
            tag_id: tag.into(),
            field,
            pattern: pattern.into(),
        };
        Classifier::new(
            &[
                tag("design", TagKind::Category),
                tag("ivi", TagKind::Project),
                tag("other", TagKind::Project),
            ],
            &[
                rule("design", RuleField::App, "com.test.figma"),
                rule("ivi", RuleField::Title, "IVI"),
                rule("other", RuleField::Title, "başka"),
            ],
        )
    }

    #[test]
    fn splits_days_hours_apps_and_titles() {
        let sessions = [
            s("figma", "IVI_Home.fig", 9 * 60 + 30, 10 * 60 + 30), // 09:30–10:30 UTC
            s(
                "teams",
                "IVI senkron",
                24 * 60 + 14 * 60,
                24 * 60 + 14 * 60 + 30,
            ), // 2. gün
            s("figma", "başka iş", 11 * 60, 12 * 60),              // başka proje
        ];
        let days = [t(0), t(24 * 60)];
        // UTC+3: 09:30 UTC → 12:30 yerel.
        let r = build(
            &sessions,
            &classifier(),
            "ivi",
            t(0),
            t(48 * 60),
            &days,
            |_| 3 * 3600,
        );
        assert_eq!(r.total_seconds, 90 * 60);
        assert_eq!(r.days, [3600, 1800]);
        assert_eq!(r.hours[12], 30 * 60);
        assert_eq!(r.hours[13], 30 * 60);
        assert_eq!(r.hours[17], 30 * 60);
        assert_eq!(r.hours.iter().sum::<i64>(), 90 * 60);
        assert_eq!(r.apps[0].app_name, "figma");
        assert_eq!(r.apps[0].seconds, 3600);
        assert_eq!(r.titles[0].title, "IVI_Home.fig");
        let cats: Vec<_> = r
            .categories
            .iter()
            .map(|b| (b.id.as_deref(), b.seconds))
            .collect();
        assert_eq!(cats, [(Some("design"), 3600), (None, 1800)]);
    }

    #[test]
    fn focus_counts_only_blocks_of_the_project() {
        let sessions = [
            s("figma", "IVI a", 0, 50),
            s("figma", "başka", 60, 100), // 10 dk mola: yeni blok, başka proje
            s("figma", "IVI b", 110, 140),
        ];
        let r = build(
            &sessions,
            &classifier(),
            "ivi",
            t(0),
            t(24 * 60),
            &[t(0)],
            |_| 0,
        );
        assert_eq!(r.blocks, 2);
        assert_eq!(r.focus_seconds, 40 * 60);
    }
}
