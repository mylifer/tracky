//! Zaman çizelgesi: günün projeye atanmış süresini firmaya gönderilecek iş kayıtlarına
//! (tarih, başlangıç, saat, tür, açıklama, taraf, birim) dönüştürür.
//!
//! Yalnızca bir projeye düşen süre iş sayılır. Aynı projede ve aynı türdeki ardışık
//! oturumlar, aradaki boşluk `MERGE_GAP`'i geçmedikçe tek kayıt olur; kaydın saati
//! boşluklar değil, oturumların toplam süresidir (yuvarlanmaz).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::Classifier;
use crate::model::Session;

/// Aynı proje ve türdeki iki oturum arasında bundan kısa boşluk kaydı bölmez.
pub const MERGE_GAP: Duration = Duration::minutes(15);
/// Bundan kısa kayıt önerilmez (bir mesaja bakmak gibi kısa geçişler).
pub const MIN_ENTRY: Duration = Duration::minutes(5);

/// Çalışma türü; şablondaki "Type" sütunu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryKind {
    /// Bilgisayarda tek başına çalışma.
    Working,
    /// Çevrim içi toplantı (Zoom, Teams, Meet…).
    Online,
    /// Yüz yüze (bilgisayar dışında; elle girilen kayıt).
    F2F,
}

impl EntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Online => "Online",
            Self::F2F => "F2F",
        }
    }
}

/// Zaman çizelgesi ayarları (bu cihazda; senkronize edilmez).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimesheetConfig {
    /// Raporun gittiği firma (örn. "Togg").
    pub company: String,
    /// "Consultant" sütunu.
    pub consultant: String,
    /// Kayıtların ekleneceği Excel dosyası.
    pub file_path: Option<String>,
    /// "Parties" varsayılanı (örn. kendi firman).
    pub default_party: String,
    /// Proje → firmadaki birim ("Togg Division") ve taraf.
    pub projects: Vec<ProjectMapping>,
    /// Çevrim içi toplantı sayılan uygulamalar (kimlik ya da exe adı, `*` öneki olabilir).
    pub meeting_apps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMapping {
    pub project_id: String,
    /// Firmadaki birim adı; boşsa proje adı kullanılır.
    pub division: String,
    /// Bu projenin "Parties" varsayılanı; boşsa genel varsayılan.
    pub party: Option<String>,
}

impl Default for TimesheetConfig {
    fn default() -> Self {
        Self {
            company: String::new(),
            consultant: String::new(),
            file_path: None,
            default_party: String::new(),
            projects: Vec::new(),
            meeting_apps: [
                "us.zoom.xos",
                "zoom.exe",
                "com.microsoft.teams*",
                "ms-teams.exe",
                "teams.exe",
                "com.cisco.webexmeetingsapp",
                "com.webex.meetingmanager",
                "webex.exe",
                "com.apple.FaceTime",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

/// Önerilen ya da düzenlenmiş bir iş kaydı.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimesheetEntry {
    pub date: NaiveDate,
    /// Yerel başlangıç saati.
    pub start: NaiveTime,
    /// Çalışılan saat (yuvarlanmamış).
    pub hours: f64,
    pub kind: EntryKind,
    pub details: String,
    pub party: String,
    pub project_id: String,
    pub division: String,
}

/// Tarayıcıda Google Meet / Teams sekmesi de toplantıdır.
const MEETING_TITLES: &[&str] = &["google meet", "meet - ", "microsoft teams", "zoom meeting"];

fn kind_of(session: &Session, config: &TimesheetConfig) -> EntryKind {
    if session.is_manual() {
        return EntryKind::F2F;
    }
    let id = session.app_id.to_lowercase();
    let title = session.title.to_lowercase();
    let meeting = config
        .meeting_apps
        .iter()
        .any(|p| crate::classify::app_matches(&p.to_lowercase(), &id))
        || MEETING_TITLES.iter().any(|t| title.contains(t));
    if meeting {
        EntryKind::Online
    } else {
        EntryKind::Working
    }
}

/// Pencere başlığından açıklama önerisi: uygulama ve tarayıcı ekleri atılır.
fn clean_title(title: &str, app_name: &str) -> String {
    let mut t = title
        .trim()
        .trim_start_matches(['●', '•', '*'])
        .trim()
        .to_string();
    for sep in [" — ", " – ", " - "] {
        for suffix in [
            app_name,
            "Figma",
            "Google Chrome",
            "Safari",
            "Microsoft Edge",
            "Visual Studio Code",
        ] {
            if let Some(rest) = t.strip_suffix(&format!("{sep}{suffix}")) {
                t = rest.trim().to_string();
            }
        }
    }
    t
}

/// Kırpılmış oturum: (başlangıç, bitiş, oturum, proje, tür).
type Span<'a> = (DateTime<Utc>, DateTime<Utc>, &'a Session, String, EntryKind);

/// Günün oturumlarından iş kaydı önerileri; başlangıca göre sıralı.
/// `day_start`/`day_end` yerel günün sınırlarıdır; oturumlar bunlara kırpılır.
pub fn propose(
    sessions: &[Session],
    classifier: &Classifier,
    project_names: &HashMap<String, String>,
    config: &TimesheetConfig,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Vec<TimesheetEntry> {
    struct Run {
        project: String,
        kind: EntryKind,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        worked: Duration,
        titles: HashMap<String, Duration>,
    }
    let mut spans: Vec<Span> = sessions
        .iter()
        .filter_map(|s| {
            let project = classifier.classify(s).project?;
            let (a, b) = (s.started_at.max(day_start), s.ended_at.min(day_end));
            (b > a).then(|| (a, b, s, project, kind_of(s, config)))
        })
        .collect();
    spans.sort_by_key(|s| s.0);

    // Her (proje, tür) için açık kayıt; araya başka iş girse de boşluk kısaysa sürer.
    let mut open: HashMap<(String, EntryKind), Run> = HashMap::new();
    let mut runs: Vec<Run> = Vec::new();
    for (a, b, s, project, kind) in spans {
        let key = (project.clone(), kind);
        if let Some(run) = open.get(&key)
            && a - run.end > MERGE_GAP
        {
            runs.push(open.remove(&key).expect("az önce bulundu"));
        }
        let run = open.entry(key).or_insert_with(|| Run {
            project,
            kind,
            start: a,
            end: a,
            worked: Duration::zero(),
            titles: HashMap::new(),
        });
        run.end = run.end.max(b);
        run.worked += b - a;
        *run.titles
            .entry(clean_title(&s.title, &s.app_name))
            .or_insert(Duration::zero()) += b - a;
    }
    runs.extend(open.into_values());

    let mut out: Vec<TimesheetEntry> = runs
        .into_iter()
        .filter(|r| r.worked >= MIN_ENTRY)
        .map(|r| {
            let mapping = config.projects.iter().find(|m| m.project_id == r.project);
            let name = project_names.get(&r.project).cloned().unwrap_or_default();
            let division = mapping
                .map(|m| m.division.trim())
                .filter(|d| !d.is_empty())
                .map_or(name, str::to_string);
            let party = mapping
                .and_then(|m| m.party.clone())
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| config.default_party.clone());
            let details = if r.kind == EntryKind::Online {
                // Toplantı uygulamasının başlığı ("Zoom Meeting") açıklama değildir.
                String::new()
            } else {
                let mut titles: Vec<_> = r
                    .titles
                    .into_iter()
                    .filter(|(t, _)| !t.is_empty())
                    .collect();
                titles.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                titles
                    .into_iter()
                    .next()
                    .map(|(t, _)| t)
                    .unwrap_or_default()
            };
            let local = r.start.with_timezone(&Local);
            TimesheetEntry {
                date: local.date_naive(),
                start: local.time().with_nanosecond(0).unwrap_or(local.time()),
                hours: r.worked.num_seconds() as f64 / 3600.0,
                kind: r.kind,
                details,
                party,
                project_id: r.project,
                division,
            }
        })
        .collect();
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.division.cmp(&b.division)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Rule, RuleField, Tag, TagKind};
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t(min: i64) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(2026, 10, 1, 9, 0, 0)
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(min)
    }

    fn s(app: &str, title: &str, from: i64, to: i64, project: Option<&str>) -> Session {
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
            project_id: project.map(Into::into),
        }
    }

    fn setup() -> (Classifier, HashMap<String, String>, TimesheetConfig) {
        let tag = |id: &str, name: &str| Tag {
            id: id.into(),
            kind: TagKind::Project,
            name: name.into(),
            color: 1,
        };
        let classifier = Classifier::new(
            &[tag("tru", "Trumore"), tag("sync", "Int.Work.Sync.")],
            &[Rule {
                id: "r".into(),
                tag_id: "tru".into(),
                field: RuleField::Title,
                pattern: "trumore".into(),
            }],
        );
        let names = HashMap::from([
            ("tru".to_string(), "Trumore".to_string()),
            ("sync".to_string(), "Int.Work.Sync.".to_string()),
        ]);
        let config = TimesheetConfig {
            default_party: "ADBA".into(),
            projects: vec![ProjectMapping {
                project_id: "sync".into(),
                division: "Int.Work.Sync.".into(),
                party: None,
            }],
            ..Default::default()
        };
        (classifier, names, config)
    }

    #[test]
    fn groups_by_project_and_kind_with_short_gaps() {
        let (classifier, names, config) = setup();
        let sessions = [
            s("Figma", "Trumore Loyalty UI/UX — Figma", 0, 50, None),
            s("Slack", "#genel", 50, 55, None), // projesiz: kayda girmez
            s("Figma", "Trumore Loyalty UI/UX — Figma", 55, 90, None), // 5 dk boşluk: aynı kayıt
            s("us.zoom.xos", "Zoom Meeting", 90, 120, Some("tru")), // toplantı: ayrı kayıt
            s("Figma", "Trumore Pitchdeck — Figma", 150, 170, None), // 60 dk boşluk: yeni kayıt
            s("Slack", "sync", 170, 173, Some("sync")), // 3 dk: çok kısa
        ];
        let got = propose(&sessions, &classifier, &names, &config, t(-540), t(900));
        let rows: Vec<_> = got
            .iter()
            .map(|e| {
                (
                    e.start.format("%H:%M").to_string(),
                    (e.hours * 60.0).round() as i64,
                    e.kind,
                    e.details.as_str(),
                    e.division.as_str(),
                    e.party.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "09:00".to_string(),
                    85,
                    EntryKind::Working,
                    "Trumore Loyalty UI/UX",
                    "Trumore",
                    "ADBA"
                ),
                (
                    "10:30".to_string(),
                    30,
                    EntryKind::Online,
                    "",
                    "Trumore",
                    "ADBA"
                ),
                (
                    "11:30".to_string(),
                    20,
                    EntryKind::Working,
                    "Trumore Pitchdeck",
                    "Trumore",
                    "ADBA"
                ),
            ]
        );
    }

    #[test]
    fn manual_entries_are_face_to_face_and_mapping_applies() {
        let (classifier, names, mut config) = setup();
        config.projects[0].party = Some("Togg".into());
        let mut meeting = s("kum.manual/Workshop", "Workshop", 0, 60, Some("sync"));
        meeting.app_name = "Workshop".into();
        let got = propose(&[meeting], &classifier, &names, &config, t(-540), t(900));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, EntryKind::F2F);
        assert_eq!(got[0].details, "Workshop");
        assert_eq!(
            (got[0].division.as_str(), got[0].party.as_str()),
            ("Int.Work.Sync.", "Togg")
        );
        assert!((got[0].hours - 1.0).abs() < 1e-9);
    }

    #[test]
    fn browser_meetings_are_online() {
        let (classifier, names, config) = setup();
        let got = propose(
            &[s("com.google.Chrome", "Meet - Trumore weekly", 0, 30, None)],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        assert_eq!(got[0].kind, EntryKind::Online);
    }
}
