//! Eğilimler: proje ve kategorilerin ardışık dönemlerdeki (haftalar) süreleri.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::classify::Classifier;
use crate::model::Session;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trends {
    /// Dönem başları (eskiden yeniye).
    pub periods: Vec<DateTime<Utc>>,
    pub categories: Vec<Series>,
    pub projects: Vec<Series>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    /// `None` = kategorisiz / projesiz.
    pub id: Option<String>,
    /// Dönem başına süre (saniye), `periods` sırasıyla.
    pub seconds: Vec<i64>,
}

/// `bounds` artan sıralı dönem sınırlarıdır (n dönem için n + 1 öğe; son öğe aralığın
/// sonu). Gece yarısı gibi dönem sınırını aşan oturum her döneme kendi payı kadar yazılır.
/// Seriler toplam süreye göre azalan sıralıdır; projesiz süre listelenmez.
pub fn trends(sessions: &[Session], classifier: &Classifier, bounds: &[DateTime<Utc>]) -> Trends {
    let n = bounds.len().saturating_sub(1);
    let mut categories: HashMap<Option<String>, Vec<i64>> = HashMap::new();
    let mut projects: HashMap<Option<String>, Vec<i64>> = HashMap::new();
    for s in sessions {
        let class = classifier.classify(s);
        for i in 0..n {
            let (a, b) = (s.started_at.max(bounds[i]), s.ended_at.min(bounds[i + 1]));
            if b <= a {
                continue;
            }
            let secs = (b - a).num_seconds();
            categories
                .entry(class.category.clone())
                .or_insert_with(|| vec![0; n])[i] += secs;
            if class.project.is_some() {
                projects
                    .entry(class.project.clone())
                    .or_insert_with(|| vec![0; n])[i] += secs;
            }
        }
    }
    let sorted = |map: HashMap<Option<String>, Vec<i64>>| {
        let mut v: Vec<Series> = map
            .into_iter()
            .map(|(id, seconds)| Series { id, seconds })
            .collect();
        v.sort_by(|a, b| {
            let total = |s: &Series| s.seconds.iter().sum::<i64>();
            total(b).cmp(&total(a)).then(a.id.cmp(&b.id))
        });
        v
    };
    Trends {
        periods: bounds[..n].to_vec(),
        categories: sorted(categories),
        projects: sorted(projects),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Rule, RuleField, Tag, TagKind};
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    fn t(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 21, 0, 0, 0).unwrap() + Duration::hours(h)
    }

    fn s(app: &str, title: &str, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
        }
    }

    #[test]
    fn splits_by_period_and_sorts_by_total() {
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
        let classifier = Classifier::new(
            &[tag("dev", TagKind::Category), tag("kum", TagKind::Project)],
            &[
                rule("dev", RuleField::App, "code"),
                rule("kum", RuleField::Title, "tracky"),
            ],
        );
        let week = 24 * 7;
        let sessions = [
            s("code", "sync.rs — tracky", 1, 3), // 1. hafta: 2 sa dev + kum
            s("code", "a.py — başka", week + 1, week + 2), // 2. hafta: 1 sa dev
            s("slack", "#tracky", week - 1, week + 1), // sınırı aşar: 1 + 1 sa kum
            s("code", "eski — tracky", -5, -1),  // aralık dışı
        ];
        let r = trends(&sessions, &classifier, &[t(0), t(week), t(2 * week)]);
        assert_eq!(r.periods, [t(0), t(week)]);
        let cat: Vec<_> = r
            .categories
            .iter()
            .map(|s| (s.id.as_deref(), s.seconds.clone()))
            .collect();
        assert_eq!(
            cat,
            [(Some("dev"), vec![7200, 3600]), (None, vec![3600, 3600])]
        );
        let proj: Vec<_> = r
            .projects
            .iter()
            .map(|s| (s.id.as_deref(), s.seconds.clone()))
            .collect();
        assert_eq!(proj, [(Some("kum"), vec![3 * 3600, 3600])]);
    }
}
