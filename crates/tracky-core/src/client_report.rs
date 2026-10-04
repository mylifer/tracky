//! Aylık müşteri raporu: projelerin gün gün saatleri (satırlar projeler, sütunlar ayın
//! günleri), müşteri onayı ve faturalama için.
//!
//! Saatler iki kaynaktan gelebilir: onaylanmış zaman çizelgesi kayıtları (firmaya yazılan,
//! yuvarlanmış saat) ya da takip edilen ve projeye düşen süre (bilgisayarlar arası
//! çakışmalar bir kez sayılarak, toplamlardaki gibi).

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::{Classifier, Client, Tag, TagKind};
use crate::model::Session;
use crate::timesheet::TimesheetEntry;

/// Saatlerin kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportSource {
    /// Zaman çizelgesi satırları.
    Timesheet,
    /// Takip edilen süre.
    Tracked,
}

/// Raporun bir satırı: bir proje (zaman çizelgesinde projesi bilinmeyen kayıtlar birimine göre).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRow {
    /// Proje kimliği; projesi bilinmeyen zaman çizelgesi kaydında boş.
    pub project_id: String,
    pub project: String,
    /// Kategorik paletteki renk yuvası (projesi bilinmeyende 0).
    pub color: u8,
    pub client: Option<String>,
    /// Gün başına saat, `days` sırasıyla.
    pub hours: Vec<f64>,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientReport {
    pub days: Vec<NaiveDate>,
    pub source: ReportSource,
    /// Dönemde (seçili müşteri için) zaman çizelgesi kaydı var mı?
    pub timesheet_available: bool,
    /// Müşteriye, sonra projeye göre sıralı; müşterisizler sonda.
    pub rows: Vec<ReportRow>,
    pub day_totals: Vec<f64>,
    pub total: f64,
}

/// Projelerin adı, rengi ve müşterisi.
#[derive(Debug, Clone, Default)]
pub struct Projects {
    by_id: HashMap<String, (String, u8, Option<Client>)>,
}

impl Projects {
    /// `project_clients`: proje → müşteri kimliği.
    pub fn new(
        tags: &[Tag],
        clients: &[Client],
        project_clients: &HashMap<String, String>,
    ) -> Self {
        let clients: HashMap<&str, &Client> = clients.iter().map(|c| (c.id.as_str(), c)).collect();
        let by_id = tags
            .iter()
            .filter(|t| t.kind == TagKind::Project)
            .map(|t| {
                let client = project_clients
                    .get(&t.id)
                    .and_then(|c| clients.get(c.as_str()))
                    .map(|c| (*c).clone());
                (t.id.clone(), (t.name.clone(), t.color, client))
            })
            .collect();
        Self { by_id }
    }

    /// Proje seçili müşterinin mi (`None`: tüm müşteriler, projesi bilinmeyenler dahil)?
    fn wanted(&self, project_id: &str, client: Option<&str>) -> bool {
        match client {
            None => true,
            Some(c) => self
                .by_id
                .get(project_id)
                .and_then(|p| p.2.as_ref())
                .is_some_and(|x| x.id == c),
        }
    }
}

/// Satırları biriktirir: anahtar → (kimlik, ad, renk, müşteri, gün başına değer).
struct Matrix<'a> {
    projects: &'a Projects,
    days: usize,
    rows: HashMap<String, ReportRow>,
}

impl<'a> Matrix<'a> {
    fn new(projects: &'a Projects, days: usize) -> Self {
        Self {
            projects,
            days,
            rows: HashMap::new(),
        }
    }

    /// `fallback`: projesi bilinmeyen kaydın satır adı.
    fn add(&mut self, project_id: &str, fallback: &str, day: usize, hours: f64) {
        let known = self.projects.by_id.get(project_id);
        let key = match known {
            Some(_) => project_id.to_string(),
            None => format!("?{fallback}"),
        };
        let days = self.days;
        let row = self.rows.entry(key).or_insert_with(|| {
            let (project, color, client) = match known {
                Some((name, color, client)) => (
                    name.clone(),
                    *color,
                    client.as_ref().map(|c| c.name.clone()),
                ),
                None => (fallback.to_string(), 0, None),
            };
            ReportRow {
                project_id: if known.is_some() {
                    project_id.to_string()
                } else {
                    String::new()
                },
                project,
                color,
                client,
                hours: vec![0.0; days],
                total: 0.0,
            }
        });
        row.hours[day] += hours;
    }

    fn finish(
        self,
        days: Vec<NaiveDate>,
        source: ReportSource,
        timesheet_available: bool,
    ) -> ClientReport {
        let mut rows: Vec<ReportRow> = self.rows.into_values().collect();
        for r in &mut rows {
            r.total = r.hours.iter().fold(0.0, |a, b| a + b);
        }
        rows.retain(|r| r.total > 0.0);
        rows.sort_by(|a, b| {
            (
                a.client.is_none(),
                a.client.as_deref().map(str::to_lowercase),
                a.project.to_lowercase(),
            )
                .cmp(&(
                    b.client.is_none(),
                    b.client.as_deref().map(str::to_lowercase),
                    b.project.to_lowercase(),
                ))
        });
        let day_totals: Vec<f64> = (0..days.len())
            .map(|d| rows.iter().fold(0.0, |a, r| a + r.hours[d]))
            .collect();
        let total = rows.iter().fold(0.0, |a, r| a + r.total);
        ClientReport {
            days,
            source,
            timesheet_available,
            rows,
            day_totals,
            total,
        }
    }
}

/// Seçili müşterinin (`None`: hepsi) `days` günlerindeki zaman çizelgesi kayıtları.
pub fn timesheet_entries_for<'e>(
    entries: &'e [TimesheetEntry],
    projects: &Projects,
    days: &[NaiveDate],
    client: Option<&str>,
) -> Vec<&'e TimesheetEntry> {
    entries
        .iter()
        .filter(|e| days.contains(&e.date) && projects.wanted(&e.project_id, client))
        .collect()
}

/// Zaman çizelgesi kayıtlarından rapor: firmaya yazılan (yuvarlanmış) saatler. Projesi
/// bilinmeyen kayıt birimi (yoksa "Projesiz") adıyla ayrı satır olur.
pub fn from_timesheet(
    entries: &[&TimesheetEntry],
    projects: &Projects,
    days: Vec<NaiveDate>,
) -> ClientReport {
    let mut m = Matrix::new(projects, days.len());
    for e in entries {
        if let Some(d) = days.iter().position(|d| *d == e.date) {
            let fallback = if e.division.trim().is_empty() {
                "Projesiz"
            } else {
                e.division.trim()
            };
            m.add(&e.project_id, fallback, d, e.hours);
        }
    }
    m.finish(days, ReportSource::Timesheet, !entries.is_empty())
}

/// Takip edilen süreden rapor. `day_starts` yerel gün sınırlarıdır (`days.len() + 1` öğe;
/// son öğe son günün bitişi); gece yarısını aşan oturum her güne kendi payı kadar yazılır.
/// `sessions` cihazlar arası birleştirilmiş çalışma oturumlarıdır.
pub fn from_sessions(
    sessions: &[Session],
    classifier: &Classifier,
    projects: &Projects,
    days: Vec<NaiveDate>,
    day_starts: &[DateTime<Utc>],
    client: Option<&str>,
    timesheet_available: bool,
) -> ClientReport {
    let mut ms: HashMap<(String, usize), i64> = HashMap::new();
    for s in sessions {
        let Some(project) = classifier.classify(s).project else {
            continue;
        };
        if !projects.by_id.contains_key(&project) || !projects.wanted(&project, client) {
            continue;
        }
        for (d, w) in day_starts.windows(2).enumerate().take(days.len()) {
            let part = (s.ended_at.min(w[1]) - s.started_at.max(w[0])).num_milliseconds();
            if part > 0 {
                *ms.entry((project.clone(), d)).or_default() += part;
            }
        }
    }
    let mut m = Matrix::new(projects, days.len());
    // Milisaniye toplanır, en sonda saate çevrilir (oturum başına yuvarlanmaz).
    for ((project, d), v) in ms {
        m.add(&project, "", d, v as f64 / 3_600_000.0);
    }
    m.finish(days, ReportSource::Tracked, timesheet_available)
}

#[cfg(feature = "store")]
impl crate::store::Store {
    /// `days` günlerinin raporu (`day_starts`: yerel gün sınırları, `days.len() + 1` öğe).
    /// `source` verilmezse dönemde zaman çizelgesi kaydı varsa o, yoksa takip edilen süre.
    pub fn client_report(
        &self,
        days: Vec<NaiveDate>,
        day_starts: &[DateTime<Utc>],
        client: Option<&str>,
        source: Option<ReportSource>,
    ) -> crate::store::Result<ClientReport> {
        let (Some(first), Some(last)) = (days.first().copied(), days.last().copied()) else {
            return Err(crate::StoreError::Invalid("rapor dönemi boş".into()));
        };
        if day_starts.len() != days.len() + 1 {
            return Err(crate::StoreError::Invalid("gün sınırları eksik".into()));
        }
        let tags = self.tags()?;
        let projects = Projects::new(&tags, &self.clients()?, &self.project_clients()?);
        let saved = self.timesheet_entries(first, last)?;
        let entries: Vec<TimesheetEntry> = saved.into_iter().map(|s| s.entry).collect();
        let entries = timesheet_entries_for(&entries, &projects, &days, client);
        let available = !entries.is_empty();
        match source.unwrap_or(if available {
            ReportSource::Timesheet
        } else {
            ReportSource::Tracked
        }) {
            ReportSource::Timesheet => Ok(from_timesheet(&entries, &projects, days)),
            ReportSource::Tracked => {
                let (from, to) = (day_starts[0], day_starts[days.len()]);
                let sessions = self.merged_sessions_between(from, to)?;
                let classifier = Classifier::new(&tags, &self.rules()?);
                Ok(from_sessions(
                    &sessions,
                    &classifier,
                    &projects,
                    days,
                    day_starts,
                    client,
                    available,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timesheet::EntryKind;
    use chrono::{Duration, NaiveTime};
    use uuid::Uuid;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    fn days() -> Vec<NaiveDate> {
        (1..=30).map(d).collect()
    }

    /// UTC gün sınırları (testte yerel saat dilimi yok).
    fn starts() -> Vec<DateTime<Utc>> {
        (0..=30)
            .map(|i| d(1).and_hms_opt(0, 0, 0).unwrap().and_utc() + Duration::days(i))
            .collect()
    }

    fn tag(id: &str, name: &str, kind: TagKind) -> Tag {
        Tag {
            id: id.into(),
            kind,
            name: name.into(),
            color: 2,
        }
    }

    fn fixture() -> (Vec<Tag>, Projects) {
        let tags = vec![
            tag("kum", "Kum", TagKind::Project),
            tag("loy", "Loyalty", TagKind::Project),
            tag("ic", "İç işler", TagKind::Project),
            tag("dev", "Geliştirme", TagKind::Category),
        ];
        let clients = vec![
            Client {
                id: "togg".into(),
                name: "Togg".into(),
            },
            Client {
                id: "adba".into(),
                name: "ADBA".into(),
            },
        ];
        let links = HashMap::from([("kum".into(), "togg".into()), ("loy".into(), "adba".into())]);
        let projects = Projects::new(&tags, &clients, &links);
        (tags, projects)
    }

    fn entry(day: u32, project: &str, hours: f64, division: &str) -> TimesheetEntry {
        TimesheetEntry {
            date: d(day),
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            hours,
            actual_hours: Some(hours - 0.1),
            kind: EntryKind::Working,
            details: String::new(),
            party: String::new(),
            project_id: project.into(),
            division: division.into(),
        }
    }

    fn session(project: Option<&str>, from: DateTime<Utc>, minutes: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: "com.apple.Safari".into(),
            app_name: "Safari".into(),
            title: "x".into(),
            url: None,
            domain: None,
            started_at: from,
            ended_at: from + Duration::minutes(minutes),
            category_id: None,
            project_id: project.map(String::from),
        }
    }

    #[test]
    fn timesheet_rows_make_a_project_by_day_matrix() {
        let (_, projects) = fixture();
        let entries = vec![
            entry(1, "kum", 2.5, ""),
            entry(1, "kum", 0.75, ""),
            entry(3, "loy", 1.0, ""),
            entry(3, "", 0.5, "Togg Genel"),
            entry(5, "kum", 4.0, ""),
            // Ay dışı: sayılmaz.
            NaiveDate::from_ymd_opt(2026, 10, 1)
                .map(|date| TimesheetEntry {
                    date,
                    ..entry(1, "kum", 9.0, "")
                })
                .unwrap(),
        ];
        let picked = timesheet_entries_for(&entries, &projects, &days(), None);
        let r = from_timesheet(&picked, &projects, days());
        assert_eq!(r.source, ReportSource::Timesheet);
        assert!(r.timesheet_available);
        // ADBA, Togg, sonra müşterisiz.
        let names: Vec<_> = r.rows.iter().map(|r| r.project.as_str()).collect();
        assert_eq!(names, ["Loyalty", "Kum", "Togg Genel"]);
        let kum = &r.rows[1];
        assert_eq!((kum.hours[0], kum.hours[4], kum.total), (3.25, 4.0, 7.25));
        assert_eq!(kum.client.as_deref(), Some("Togg"));
        assert_eq!(r.rows[2].project_id, "");
        assert_eq!(r.day_totals[2], 1.5);
        assert_eq!(r.total, 8.75);

        // Müşteri seçilince yalnızca onun projeleri; projesi bilinmeyenler girmez.
        let picked = timesheet_entries_for(&entries, &projects, &days(), Some("togg"));
        let r = from_timesheet(&picked, &projects, days());
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.total, 7.25);
        let picked = timesheet_entries_for(&entries, &projects, &days(), Some("yok"));
        assert!(!from_timesheet(&picked, &projects, days()).timesheet_available);
    }

    #[test]
    fn tracked_time_is_split_at_midnight_and_filtered_by_client() {
        let (tags, projects) = fixture();
        let classifier = Classifier::new(&tags, &[]);
        let at = |day: u32, h: u32| d(day).and_hms_opt(h, 0, 0).unwrap().and_utc();
        let sessions = vec![
            session(Some("kum"), at(1, 9), 90),
            // 2 Eyl 23:00 – 3 Eyl 01:00: iki güne birer saat.
            session(Some("kum"), at(2, 23), 120),
            session(Some("loy"), at(4, 10), 45),
            session(Some("ic"), at(4, 12), 30),
            session(None, at(5, 10), 60),
        ];
        let r = from_sessions(
            &sessions,
            &classifier,
            &projects,
            days(),
            &starts(),
            None,
            false,
        );
        assert_eq!(r.source, ReportSource::Tracked);
        let names: Vec<_> = r.rows.iter().map(|r| r.project.as_str()).collect();
        assert_eq!(names, ["Loyalty", "Kum", "İç işler"]);
        let kum = &r.rows[1];
        assert_eq!((kum.hours[0], kum.hours[1], kum.hours[2]), (1.5, 1.0, 1.0));
        assert_eq!(kum.total, 3.5);
        assert_eq!(r.day_totals[3], 1.25);
        assert_eq!(r.total, 4.75);

        let r = from_sessions(
            &sessions,
            &classifier,
            &projects,
            days(),
            &starts(),
            Some("adba"),
            true,
        );
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.total, 0.75);
        assert!(r.timesheet_available);
    }

    #[cfg(feature = "store")]
    #[test]
    fn store_picks_timesheet_when_available() {
        use crate::store::Store;
        let store = Store::open_in_memory().unwrap();
        let (tags, _) = fixture();
        for (i, t) in tags.iter().enumerate() {
            store.upsert_tag(t, i as i64).unwrap();
        }
        store
            .upsert_client(
                &Client {
                    id: "togg".into(),
                    name: "Togg".into(),
                },
                0,
            )
            .unwrap();
        store.set_project_client("kum", Some("togg")).unwrap();
        let at = d(2).and_hms_opt(9, 0, 0).unwrap().and_utc();
        let s = session(None, at, 60);
        store.upsert_session(&s).unwrap();
        store.set_project_for(&[s.id], Some("kum")).unwrap();

        let r = store
            .client_report(days(), &starts(), Some("togg"), None)
            .unwrap();
        assert_eq!(
            (r.source, r.timesheet_available, r.total),
            (ReportSource::Tracked, false, 1.0)
        );

        store
            .replace_timesheet_day(d(2), &[entry(2, "kum", 1.25, "")])
            .unwrap();
        let r = store
            .client_report(days(), &starts(), Some("togg"), None)
            .unwrap();
        assert_eq!((r.source, r.total), (ReportSource::Timesheet, 1.25));
        // Kullanıcı takip edilen süreye geçebilir.
        let r = store
            .client_report(days(), &starts(), Some("togg"), Some(ReportSource::Tracked))
            .unwrap();
        assert_eq!(
            (r.source, r.timesheet_available, r.total),
            (ReportSource::Tracked, true, 1.0)
        );

        assert!(
            store
                .client_report(days(), &starts()[..3], None, None)
                .is_err()
        );
    }
}
