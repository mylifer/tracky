//! Arama: pencere başlığında ya da uygulama adında bir ifade geçen süre.
//!
//! "Bu ay tracky üzerinde ne kadar çalıştım?" sorusunun yanıtı: toplam, günlere
//! dağılım, uygulamalar ve en çok geçen başlıklar. Büyük/küçük harf ve Türkçe
//! noktalı/noktasız i farkı gözetilmez ("istanbul" ⇔ "İSTANBUL", "issue" ⇔ "ISSUE").

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::Session;

/// En çok bu kadar başlık listelenir.
const MAX_TITLES: usize = 30;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub total_seconds: i64,
    /// `day_starts` sırasıyla her günün süresi.
    pub days: Vec<i64>,
    pub apps: Vec<SearchApp>,
    pub titles: Vec<SearchTitle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchApp {
    pub app_id: String,
    pub app_name: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTitle {
    pub app_name: String,
    pub title: String,
    pub seconds: i64,
}

/// Karşılaştırma için: küçük harf, Türkçe I/İ/ı hepsi "i".
pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'İ' | 'I' | 'ı' => 'i',
            c => c,
        })
        .flat_map(char::to_lowercase)
        // "İ".to_lowercase() birleşik nokta ekler; yukarıda zaten "i" yapıldı ama
        // başka kaynaklı birleşik noktalar da eşleşmeyi bozmasın.
        .filter(|c| *c != '\u{307}')
        .collect()
}

/// Oturumun başlığında ya da uygulama adında `needle` (önceden `fold` edilmiş) geçiyor mu?
pub fn matches(session: &Session, needle: &str) -> bool {
    fold(&session.title).contains(needle) || fold(&session.app_name).contains(needle)
}

/// `[from, to)` aralığında `query` geçen oturumların dökümü. `day_starts` artan
/// sıralı yerel gün başlarıdır (ilk öğe `from`). Boş sorgu boş sonuç verir.
pub fn search(
    sessions: &[Session],
    query: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    day_starts: &[DateTime<Utc>],
) -> SearchResult {
    let needle = fold(query.trim());
    let mut result = SearchResult {
        days: vec![0; day_starts.len()],
        ..Default::default()
    };
    if needle.is_empty() {
        return result;
    }
    let mut apps: HashMap<&str, (&str, i64)> = HashMap::new();
    let mut titles: HashMap<(&str, &str), i64> = HashMap::new();
    for s in sessions {
        if !matches(s, &needle) {
            continue;
        }
        let (start, end) = (s.started_at.max(from), s.ended_at.min(to));
        let secs = (end - start).num_seconds();
        if secs <= 0 {
            continue;
        }
        result.total_seconds += secs;
        // Gece yarısını aşan oturum günlere bölünür.
        for (i, &day) in day_starts.iter().enumerate() {
            let next = day_starts.get(i + 1).copied().unwrap_or(to);
            let part = (end.min(next) - start.max(day)).num_seconds();
            if part > 0 {
                result.days[i] += part;
            }
        }
        apps.entry(&s.app_id).or_insert((&s.app_name, 0)).1 += secs;
        *titles.entry((&s.app_name, &s.title)).or_default() += secs;
    }
    result.apps = apps
        .into_iter()
        .map(|(id, (name, seconds))| SearchApp {
            app_id: id.to_string(),
            app_name: name.to_string(),
            seconds,
        })
        .collect();
    result
        .apps
        .sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));
    result.titles = titles
        .into_iter()
        .map(|((app, title), seconds)| SearchTitle {
            app_name: app.to_string(),
            title: title.to_string(),
            seconds,
        })
        .collect();
    result
        .titles
        .sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.title.cmp(&b.title)));
    result.titles.truncate(MAX_TITLES);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use uuid::Uuid;

    fn t(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap() + Duration::minutes(h * 60)
    }

    fn s(app: &str, title: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: from,
            ended_at: to,
            category_id: None,
            project_id: None,
        }
    }

    #[test]
    fn folds_case_and_turkish_i() {
        assert_eq!(fold("İSTANBUL Issue ılık"), "istanbul issue ilik");
        assert!(fold("Fix ISSUE #3").contains(&fold("issue")));
        assert!(fold("istanbul").contains(&fold("İstanbul")));
    }

    #[test]
    fn totals_days_apps_and_titles() {
        let sessions = [
            s("Code", "sync.rs — tracky", t(9), t(10)),
            s(
                "Chrome",
                "mylifer/tracky · Pull Request",
                t(11),
                t(11) + Duration::minutes(30),
            ),
            s("Slack", "#genel", t(12), t(13)),
            // Gece yarısını aşar: 23:00–01:00 → iki güne birer saat.
            s("Terminal", "TRACKY — zsh", t(23), t(25)),
            // Aralığın dışında kalan kısım sayılmaz.
            s("Code", "eski — tracky", t(-2), t(1)),
        ];
        let starts = [t(0), t(24)];
        let r = search(&sessions, " Tracky ", t(0), t(48), &starts);
        assert_eq!(r.total_seconds, (1 + 1 + 2) * 3600 + 30 * 60);
        assert_eq!(r.days, [(1 + 1 + 1) * 3600 + 30 * 60, 3600]);
        let apps: Vec<_> = r
            .apps
            .iter()
            .map(|a| (a.app_name.as_str(), a.seconds))
            .collect();
        assert_eq!(apps, [("Code", 7200), ("Terminal", 7200), ("Chrome", 1800)]);
        assert_eq!(r.titles[0].seconds, 7200);
        assert_eq!(r.titles.len(), 4);
    }

    #[test]
    fn matches_app_name_and_ignores_empty_query() {
        let sessions = [s("Slack", "#genel", t(1), t(2))];
        assert_eq!(
            search(&sessions, "slack", t(0), t(24), &[t(0)]).total_seconds,
            3600
        );
        let empty = search(&sessions, "  ", t(0), t(24), &[t(0)]);
        assert_eq!((empty.total_seconds, empty.days), (0, vec![0]));
    }
}
