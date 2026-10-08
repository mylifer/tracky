//! Oturumlardan gün/hafta raporu üretimi (saf hesaplama; depolama bağımsız).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::blocks::{self, Activity, WorkStats};
use crate::classify::{Classifier, Tag};
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
    /// Pencerenin (kurala ya da elle atamaya göre) projesi.
    pub project_id: Option<String>,
    /// Tarayıcıdaysa sitenin alan adı.
    pub domain: Option<String>,
}

/// Bilgisayardan uzakta geçen, henüz bir işe atanmamış süre (takvimde "Boşta").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleSpan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
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
    /// Tüm aralığın blok ve mola analizi (bloklar yalnızca zaman çizelgesi istenince doldurulur).
    pub work: WorkStats,
    /// Atanmamış boşta süre; çalışma toplamlarına girmez.
    pub idle_seconds: i64,
    /// Atanmamış boşta aralıklar (yalnızca zaman çizelgesi istenince).
    pub idle: Vec<IdleSpan>,
    /// Aralıkta çalışılan bilgisayarlar, süreye göre (filtre uygulanmamış; yalnızca birden
    /// çok bilgisayar varsa dolu, `Store::report_for_device`).
    pub devices: Vec<DeviceTotal>,
}

/// Aralıkta bir bilgisayarın çalışma süresi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTotal {
    pub id: String,
    pub name: String,
    pub os: String,
    /// Model ailesi ("Mac Studio" ...); bilinmiyorsa boş.
    pub model: String,
    pub seconds: i64,
    /// Bu bilgisayar mı?
    pub current: bool,
}

/// Kırpılmış oturum dilimi: (başlangıç, bitiş, oturum, kategori, proje).
type Span<'a> = (
    DateTime<Utc>,
    DateTime<Utc>,
    &'a Session,
    Option<String>,
    Option<String>,
);

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
    // Süreler milisaniye olarak toplanır, en sonda saniyeye çevrilir (oturum başına kırpılmaz).
    let mut categories: HashMap<Option<String>, i64> = HashMap::new();
    let mut projects: HashMap<Option<String>, i64> = HashMap::new();
    let mut apps: HashMap<&str, (String, i64)> = HashMap::new();
    let mut days: Vec<(i64, HashMap<Option<String>, i64>)> =
        day_starts.iter().map(|_| (0, HashMap::new())).collect();
    let mut timeline: Vec<Segment> = Vec::new();
    let mut windows: Vec<WindowSpan> = Vec::new();
    let mut total = 0;
    // Blok analizi için kırpılmış etkinlikler: (başlangıç, bitiş, oturum, kategori, proje).
    let mut spans: Vec<Span> = Vec::new();
    let mut idle: Vec<IdleSpan> = Vec::new();
    let mut idle_ms = 0;

    for s in sessions {
        let (start, end) = (s.started_at.max(from), s.ended_at.min(to));
        if end <= start {
            continue;
        }
        let ms = (end - start).num_milliseconds();
        if !s.counts_as_work() {
            idle_ms += ms;
            if with_timeline {
                idle.push(IdleSpan { start, end });
            }
            continue;
        }
        let class = classifier.classify(s);
        spans.push((start, end, s, class.category.clone(), class.project.clone()));
        total += ms;
        *categories.entry(class.category.clone()).or_default() += ms;
        *projects.entry(class.project.clone()).or_default() += ms;
        let app = apps
            .entry(s.app_id.as_str())
            .or_insert_with(|| (s.app_name.clone(), 0));
        app.1 += ms;

        // Gün sınırını aşan oturum her güne kendi payı kadar yazılır.
        for (i, day_start) in day_starts.iter().enumerate() {
            let day_end = day_starts.get(i + 1).copied().unwrap_or(to);
            let (a, b) = (start.max(*day_start), end.min(day_end));
            if b > a {
                let d = (b - a).num_milliseconds();
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
                        && last.project_id == class.project
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
                    project_id: class.project.clone(),
                    domain: s.domain.clone(),
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

    let mut per_day: Vec<WorkStats> = day_starts
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let end = day_starts.get(i + 1).copied().unwrap_or(to);
            blocks::analyze(&activities(&spans, *start, end))
        })
        .collect();
    if per_day.is_empty() {
        per_day.push(blocks::analyze(&activities(&spans, from, to)));
    }

    let mut apps: Vec<AppBucket> = apps
        .into_iter()
        .map(|(app_id, (app_name, ms))| AppBucket {
            category_id: classifier.app_category(app_id),
            app_id: app_id.to_string(),
            app_name,
            seconds: ms / 1000,
        })
        .collect();
    apps.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));

    Report {
        from: Some(from),
        to: Some(to),
        total_seconds: total / 1000,
        categories: sorted(categories),
        projects: sorted(projects),
        apps,
        days: day_starts
            .iter()
            .zip(days)
            .map(|(start, (ms, cats))| DayBucket {
                start: *start,
                seconds: ms / 1000,
                categories: sorted(cats),
            })
            .collect(),
        timeline,
        windows,
        tags: tags.to_vec(),
        work: {
            // Çok günlü aralıkta günler ayrı analiz edilip birleştirilir
            // (gece boşlukları mola sayılmasın, bloklar gün sınırında bölünsün).
            let mut stats = if per_day.len() == 1 {
                per_day.pop().unwrap_or_default()
            } else {
                blocks::merge(per_day)
            };
            if !with_timeline {
                stats.blocks.clear();
            }
            stats
        },
        idle_seconds: idle_ms / 1000,
        idle,
        devices: Vec::new(),
    }
}

/// `[from, to)` aralığına kırpılmış etkinlikler.
fn activities<'a>(
    spans: &'a [Span<'a>],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<Activity<'a>> {
    spans
        .iter()
        .filter_map(|(start, end, s, category, project)| {
            let (a, b) = ((*start).max(from), (*end).min(to));
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
        .collect()
}

/// Milisaniyeleri saniyeye çevirir; süreye göre azalan, eşitlikte kimliğe göre (kararlı
/// çıktı için).
fn sorted(map: HashMap<Option<String>, i64>) -> Vec<Bucket> {
    let mut v: Vec<Bucket> = map
        .into_iter()
        .map(|(id, ms)| Bucket {
            id,
            seconds: ms / 1000,
        })
        .filter(|b| b.seconds > 0)
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
            project_id: None,
            block_from: None,
        }
    }

    #[test]
    fn sums_milliseconds_before_rounding() {
        // Saniyenin altı kesirler oturum başına atılmaz: 4 × 1,5 sn = 6 sn (4 değil).
        let mut sessions: Vec<Session> = (0..4)
            .map(|i| {
                let mut x = s("Code", "a", i * 10, i * 10);
                x.ended_at = x.started_at + Duration::milliseconds(1500);
                x
            })
            .collect();
        for a in [100, 110] {
            let mut away = Session::idle(t(a), t(a));
            away.ended_at = away.started_at + Duration::milliseconds(2500);
            sessions.push(away);
        }
        let c = Classifier::new(&[], &[]);
        let r = build(&sessions, &[], &c, t(0), t(1000), &[t(0)], false);
        assert_eq!(r.total_seconds, 6);
        assert_eq!(r.apps[0].seconds, 6);
        assert_eq!(r.categories[0].seconds, 6);
        assert_eq!(r.days[0].seconds, 6);
        assert_eq!(r.idle_seconds, 5);
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
        assert_eq!(r.work.blocks.len(), 3);
        assert_eq!(r.work.switches, 1);
        assert_eq!((r.timeline[0].start, r.timeline[0].end), (t(0), t(150)));
    }

    #[test]
    fn unassigned_idle_is_listed_apart_and_assigned_idle_is_work() {
        let c = Classifier::new(&[], &[]);
        let away = Session::idle(t(100), t(700));
        let mut meeting = Session::idle(t(800), t(1000));
        meeting.project_id = Some("kum".into());
        let sessions = [s("Code", "", 0, 100), away, meeting];
        let r = build(&sessions, &[], &c, t(0), t(2000), &[t(0)], true);
        assert_eq!(r.total_seconds, 100 + 200);
        assert_eq!(r.idle_seconds, 600);
        assert_eq!(
            r.idle,
            [IdleSpan {
                start: t(100),
                end: t(700)
            }]
        );
        assert!(r.apps.iter().all(|a| a.seconds != 600));
        // Zaman çizelgesi istenmezse aralıklar yok, toplam var.
        let r = build(&sessions, &[], &c, t(0), t(2000), &[t(0)], false);
        assert!(r.idle.is_empty());
        assert_eq!(r.idle_seconds, 600);
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

    #[test]
    fn windows_split_by_project_and_carry_the_site() {
        let mut a = s("Chrome", "LOY-214 · Jira", 0, 60);
        a.domain = Some("jira.firma.com".into());
        a.project_id = Some("kum".into());
        let b = s("Chrome", "LOY-214 · Jira", 61, 120);
        let tags = [Tag {
            id: "kum".into(),
            kind: TagKind::Project,
            name: "Kum".into(),
            color: 1,
        }];
        let c = Classifier::new(&tags, &[]);
        let r = build(&[a, b], &tags, &c, t(0), t(3600), &[t(0)], true);
        // Aynı pencere ama biri elle projeye atanmış: ayrı satır olur.
        let w: Vec<_> = r
            .windows
            .iter()
            .map(|w| (w.project_id.as_deref(), w.domain.as_deref()))
            .collect();
        assert_eq!(w, [(Some("kum"), Some("jira.firma.com")), (None, None)]);
    }
}
