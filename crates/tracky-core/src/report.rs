//! Oturumlardan gün/hafta raporu üretimi (saf hesaplama; depolama bağımsız).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::classify::{Classifier, Tag};
use crate::focus::{self, Activity, FocusStats};
use crate::model::Session;

/// Bir kategori ya da proje için toplam. `id: None` = kategorisiz / projesiz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub id: Option<String>,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBucket {
    pub app_id: String,
    pub app_name: String,
    /// Uygulamanın kendi kategorisi (uygulama kuralına göre).
    pub category_id: Option<String>,
    pub seconds: i64,
}

/// Bir günün toplamı ve kategori kırılımı (haftalık grafik için).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayBucket {
    pub start: DateTime<Utc>,
    pub seconds: i64,
    pub categories: Vec<Bucket>,
    pub focus_score: u8,
    pub focus_seconds: i64,
}

/// Zaman çizelgesindeki kesintisiz bir blok.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub app_name: String,
    pub title: String,
    pub category_id: Option<String>,
}

/// Uygulama çizelgesi için: aynı uygulama ve pencere başlığında kesintisiz geçen süre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSpan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub category_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub total_seconds: i64,
    pub categories: Vec<Bucket>,
    pub projects: Vec<Bucket>,
    pub apps: Vec<AppBucket>,
    pub days: Vec<DayBucket>,
    pub timeline: Vec<Segment>,
    /// Pencere başlığı düzeyinde zaman çizelgesi (yalnızca zaman çizelgesi istenince).
    pub windows: Vec<WindowSpan>,
    pub tags: Vec<Tag>,
    /// Tüm aralığın odak analizi (bloklar yalnızca zaman çizelgesi istenince doldurulur).
    pub focus: FocusStats,
    /// Aralıktaki odak zamanlayıcıları (zaman çizelgesiyle birlikte).
    pub focus_timers: Vec<crate::store::FocusTimer>,
}

/// Kırpılmış oturum dilimi: (başlangıç, bitiş, oturum, kategori).
type Span<'a> = (DateTime<Utc>, DateTime<Utc>, &'a Session, Option<String>);

/// Aynı uygulamanın bu kadar yakın bloklarını zaman çizelgesinde birleştir.
const MERGE_GAP_SECS: i64 = 5;

/// `[from, to)` aralığının raporu. `day_starts` yerel gün sınırlarıdır
/// (artan sırada, ilk öğe `from`); saat dilimi çağıranın sorumluluğundadır.
pub fn build(
    sessions: &[Session],
    tags: &[Tag],
    classifier: &Classifier,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    day_starts: &[DateTime<Utc>],
    with_timeline: bool,
) -> Report {
    let mut categories: HashMap<Option<String>, i64> = HashMap::new();
    let mut projects: HashMap<Option<String>, i64> = HashMap::new();
    let mut apps: HashMap<&str, (String, i64)> = HashMap::new();
    let mut days: Vec<(i64, HashMap<Option<String>, i64>)> =
        day_starts.iter().map(|_| (0, HashMap::new())).collect();
    let mut timeline: Vec<Segment> = Vec::new();
    let mut windows: Vec<WindowSpan> = Vec::new();
    let mut total = 0;
    // Odak analizi için kırpılmış etkinlikler: (başlangıç, bitiş, oturum, kategori).
    let mut spans: Vec<Span> = Vec::new();

    for s in sessions {
        let (start, end) = (s.started_at.max(from), s.ended_at.min(to));
        if end <= start {
            continue;
        }
        let secs = (end - start).num_seconds();
        let class = classifier.classify(s);
        spans.push((start, end, s, class.category.clone()));
        total += secs;
        *categories.entry(class.category.clone()).or_default() += secs;
        *projects.entry(class.project.clone()).or_default() += secs;
        let app = apps
            .entry(s.app_id.as_str())
            .or_insert_with(|| (s.app_name.clone(), 0));
        app.1 += secs;

        // Gün sınırını aşan oturum her güne kendi payı kadar yazılır.
        for (i, day_start) in day_starts.iter().enumerate() {
            let day_end = day_starts.get(i + 1).copied().unwrap_or(to);
            let (a, b) = (start.max(*day_start), end.min(day_end));
            if b > a {
                let d = (b - a).num_seconds();
                days[i].0 += d;
                *days[i].1.entry(class.category.clone()).or_default() += d;
            }
        }

        if with_timeline {
            match windows.last_mut() {
                Some(last)
                    if last.app_id == s.app_id
                        && last.title == s.title
                        && last.category_id == class.category
                        && start - last.end <= Duration::seconds(MERGE_GAP_SECS) =>
                {
                    last.end = last.end.max(end);
                }
                _ => windows.push(WindowSpan {
                    start,
                    end,
                    app_id: s.app_id.clone(),
                    app_name: s.app_name.clone(),
                    title: s.title.clone(),
                    category_id: class.category.clone(),
                }),
            }
            match timeline.last_mut() {
                Some(last)
                    if last.app_name == s.app_name
                        && last.category_id == class.category
                        && start - last.end <= Duration::seconds(MERGE_GAP_SECS) =>
                {
                    last.end = last.end.max(end);
                }
                _ => timeline.push(Segment {
                    start,
                    end,
                    app_name: s.app_name.clone(),
                    title: s.title.clone(),
                    category_id: class.category,
                }),
            }
        }
    }

    let mut per_day: Vec<FocusStats> = day_starts
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let end = day_starts.get(i + 1).copied().unwrap_or(to);
            focus::analyze(&activities(&spans, *start, end))
        })
        .collect();
    if per_day.is_empty() {
        per_day.push(focus::analyze(&activities(&spans, from, to)));
    }

    let mut apps: Vec<AppBucket> = apps
        .into_iter()
        .map(|(app_id, (app_name, seconds))| AppBucket {
            category_id: classifier.app_category(app_id),
            app_id: app_id.to_string(),
            app_name,
            seconds,
        })
        .collect();
    apps.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));

    Report {
        from: Some(from),
        to: Some(to),
        total_seconds: total,
        categories: sorted(categories),
        projects: sorted(projects),
        apps,
        days: day_starts
            .iter()
            .zip(days)
            .zip(&per_day)
            .map(|((start, (seconds, cats)), day)| DayBucket {
                start: *start,
                seconds,
                categories: sorted(cats),
                focus_score: day.score,
                focus_seconds: day.focus_seconds,
            })
            .collect(),
        timeline,
        windows,
        tags: tags.to_vec(),
        focus: {
            // Çok günlü aralıkta günler ayrı analiz edilip birleştirilir
            // (gece boşlukları mola sayılmasın, skor gün gün hesaplansın).
            let mut stats = if per_day.len() == 1 {
                per_day.pop().unwrap_or_default()
            } else {
                focus::merge(per_day)
            };
            if !with_timeline {
                stats.blocks.clear();
            }
            stats
        },
        focus_timers: Vec::new(),
    }
}

/// `[from, to)` aralığına kırpılmış odak etkinlikleri.
fn activities<'a>(
    spans: &'a [Span<'a>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<Activity<'a>> {
    spans
        .iter()
        .filter_map(|(start, end, s, category)| {
            let (a, b) = ((*start).max(from), (*end).min(to));
            (b > a).then(|| Activity {
                start: a,
                end: b,
                app_id: &s.app_id,
                app_name: &s.app_name,
                category: category.as_deref(),
            })
        })
        .collect()
}

/// Süreye göre azalan; eşitlikte kimliğe göre (kararlı çıktı için).
fn sorted(map: HashMap<Option<String>, i64>) -> Vec<Bucket> {
    let mut v: Vec<Bucket> = map
        .into_iter()
        .filter(|(_, s)| *s > 0)
        .map(|(id, seconds)| Bucket { id, seconds })
        .collect();
    v.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.id.cmp(&b.id)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Rule, RuleField, TagKind};
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn s(app: &str, title: &str, a: i64, b: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(a),
            ended_at: t(b),
            category_id: None,
        }
    }

    #[test]
    fn aggregates_by_category_project_app_and_day() {
        let tags = vec![
            Tag {
                id: "dev".into(),
                kind: TagKind::Category,
                name: "Geliştirme".into(),
                color: 1,
            },
            Tag {
                id: "kum".into(),
                kind: TagKind::Project,
                name: "Kum".into(),
                color: 2,
            },
        ];
        let rules = vec![
            Rule {
                id: "1".into(),
                tag_id: "dev".into(),
                field: RuleField::App,
                pattern: "com.test.Code".into(),
            },
            Rule {
                id: "2".into(),
                tag_id: "kum".into(),
                field: RuleField::Title,
                pattern: "kum".into(),
            },
        ];
        let c = Classifier::new(&tags, &rules);
        let sessions = [
            s("Code", "kum/main.rs", 0, 100),
            s("Code", "kum/lib.rs", 102, 150), // birleşir (aynı uygulama, 2 sn boşluk)
            s("Safari", "Haberler", 150, 200),
            s("Code", "diğer", 980, 1100), // gün sınırını (1000) aşar
        ];
        let r = build(&sessions, &tags, &c, t(0), t(2000), &[t(0), t(1000)], true);

        assert_eq!(r.total_seconds, 100 + 48 + 50 + 120);
        assert_eq!(
            r.categories,
            [
                Bucket {
                    id: Some("dev".into()),
                    seconds: 268
                },
                Bucket {
                    id: None,
                    seconds: 50
                }
            ]
        );
        assert_eq!(r.projects[0].id, None);
        assert_eq!(r.projects[1].seconds, 148);
        assert_eq!(r.apps[0].app_name, "Code");
        assert_eq!(r.apps[0].category_id.as_deref(), Some("dev"));
        assert_eq!(r.days[0].seconds, 100 + 48 + 50 + 20);
        assert_eq!(r.days[1].seconds, 100);
        assert_eq!(r.timeline.len(), 3);
        // Gün sınırını aşan oturum her gün ayrı blok olur.
        assert_eq!(r.focus.blocks.len(), 3);
        assert_eq!(r.focus.switches, 1);
        assert_eq!((r.timeline[0].start, r.timeline[0].end), (t(0), t(150)));
    }

    #[test]
    fn clips_sessions_to_range() {
        let c = Classifier::new(&[], &[]);
        let r = build(
            &[s("A", "", 0, 100)],
            &[],
            &c,
            t(50),
            t(80),
            &[t(50)],
            false,
        );
        assert_eq!(r.total_seconds, 30);
        assert!(r.timeline.is_empty());
    }

    #[test]
    fn windows_keep_title_changes_that_the_timeline_merges() {
        let sessions = vec![
            s("Code", "a.rs", 0, 60),
            s("Code", "b.rs", 61, 120),
            s("Code", "b.rs", 121, 180),
            s("Mail", "Gelen", 180, 240),
        ];
        let c = Classifier::new(&[], &[]);
        let r = build(&sessions, &[], &c, t(0), t(3600), &[t(0)], true);
        assert_eq!(r.timeline.len(), 2);
        let w: Vec<_> = r
            .windows
            .iter()
            .map(|w| (w.title.as_str(), w.start, w.end))
            .collect();
        assert_eq!(
            w,
            [
                ("a.rs", t(0), t(60)),
                ("b.rs", t(61), t(180)),
                ("Gelen", t(180), t(240))
            ]
        );
        assert!(
            build(&sessions, &[], &c, t(0), t(3600), &[t(0)], false)
                .windows
                .is_empty()
        );
    }
}
