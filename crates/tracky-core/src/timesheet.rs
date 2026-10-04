//! Zaman çizelgesi: günün projeye atanmış süresini firmaya gönderilecek iş kayıtlarına
//! (tarih, başlangıç, saat, tür, açıklama, taraf, birim) dönüştürür.
//!
//! Yalnızca bir projeye düşen süre iş sayılır. Aynı projede ve aynı türdeki ardışık
//! oturumlar, aradaki boşluk `MERGE_GAP`'i geçmedikçe tek kayıt olur; kaydın gerçek süresi
//! boşluklar değil, oturumların toplam süresidir. Firmaya giden saat bu sürenin çeyrek saate
//! yuvarlanmışıdır ([`round_quarter`]); gerçek süre kayıtta ayrıca saklanır.
//!
//! Takvimden gelen ve bir projeye düşen toplantılar kendi kaydı olur (konusu açıklama,
//! çevrim içiyse Online, değilse F2F). Toplantı süresince takip edilen iş sayılmaz: aynı
//! saat iki kez yazılmasın.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::Classifier;
use crate::model::Session;

/// Aynı proje ve türdeki iki oturum arasında bundan kısa boşluk kaydı bölmez.
pub const MERGE_GAP: Duration = Duration::minutes(15);
/// Bundan kısa kayıt önerilmez (bir mesaja bakmak gibi kısa geçişler).
pub const MIN_ENTRY: Duration = Duration::minutes(5);
/// Aktarılmış bir kaydın ardından kalan süre bundan kısaysa önerilmez: aktarırken saati
/// yuvarlamaktan (0,83 → 0,75) kalan birkaç dakika yeni iş değildir.
pub const MIN_REMAINDER: Duration = Duration::minutes(15);

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
    /// Doluysa kayıtlar Excel yerine bu Google Sheets tablosuna, tabloya eklenen Apps Script
    /// web uygulaması (`…/exec`) üzerinden yazılır.
    pub sheet_url: Option<String>,
    /// Tablonun kendisi (docs.google.com bağlantısı; açmak için).
    pub sheet_link: Option<String>,
    /// Apps Script'in yalnızca Kum'dan gelen istekleri kabul etmesi için anahtar.
    pub sheet_token: String,
    /// "Parties" varsayılanı (örn. kendi firman).
    pub default_party: String,
    /// Proje → firmadaki birim ("Togg Division") ve taraf.
    pub projects: Vec<ProjectMapping>,
    /// Çevrim içi toplantı sayılan uygulamalar (kimlik ya da exe adı, `*` öneki olabilir).
    pub meeting_apps: Vec<String>,
    /// Bir adam-günün saati (adam-gün = saat / bu değer).
    pub day_hours: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMapping {
    pub project_id: String,
    /// Firmadaki birim adı; boşsa proje adı kullanılır.
    pub division: String,
    /// Bu projenin "Parties" varsayılanı; boşsa genel varsayılan.
    pub party: Option<String>,
    /// Hazır açıklama: önerinin başlıklardan açıklaması çıkmazsa bu yazılır.
    #[serde(default)]
    pub default_details: Option<String>,
}

impl Default for TimesheetConfig {
    fn default() -> Self {
        Self {
            company: String::new(),
            consultant: String::new(),
            file_path: None,
            sheet_url: None,
            sheet_link: None,
            sheet_token: String::new(),
            default_party: String::new(),
            projects: Vec::new(),
            day_hours: 8.0,
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
    /// Firmaya yazılan saat (önerilerde çeyrek saate yuvarlanmış; elle değiştirilebilir).
    pub hours: f64,
    /// Takip edilen gerçek süre (saat, yuvarlanmamış); elle eklenen satırda yok.
    #[serde(default)]
    pub actual_hours: Option<f64>,
    pub kind: EntryKind,
    pub details: String,
    pub party: String,
    pub project_id: String,
    pub division: String,
}

impl TimesheetEntry {
    /// Gerçek süre; bilinmiyorsa yazılan saat.
    pub fn worked(&self) -> f64 {
        self.actual_hours.unwrap_or(self.hours)
    }
}

/// Saati en yakın çeyrek saate yuvarlar (en az 0,25).
pub fn round_quarter(hours: f64) -> f64 {
    ((hours * 4.0).round() / 4.0).max(0.25)
}

/// Takvimden (Outlook) bir toplantı; tekrarlayanların her biri ayrı.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    /// Takvimdeki kimlik; tekrarlayan toplantının hepsinde aynı.
    pub uid: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub subject: String,
    pub location: String,
    /// Teams, Zoom, Meet… bağlantısı var.
    pub online: bool,
    /// Düzenleyenin e-posta adresi (küçük harf). Outlook yayımlanan takvime katılımcıları
    /// yalnızca "Tüm ayrıntılar" düzeyinde yazar; çoğu zaman yoktur. Arayüze gönderilmez.
    #[serde(skip)]
    pub organizer: Option<String>,
    /// Katılımcıların e-posta adresleri (küçük harf, düzenleyen dahil olabilir).
    #[serde(skip)]
    pub attendees: Vec<String>,
}

/// Toplantıların proje kuralları bu uygulama kimliğiyle denenir (yalnızca başlık kuralları uyar).
pub const CALENDAR_APP_ID: &str = "kum.calendar";

/// Bir toplantının zaman çizelgesindeki yeri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingProject {
    Project(String),
    /// Kullanıcı bu toplantıyı (serisini) zaman çizelgesine almamayı seçti.
    Ignored,
    /// Ne elle atandı ne de bir proje kuralına uyuyor.
    Unassigned,
}

/// Elle atama (toplantı serisi → proje, `None`: yoksay) önce gelir; yoksa konusu proje
/// kurallarına (örn. başlıkta "Togg") göre sınıflandırılır.
pub fn meeting_project(
    meeting: &Meeting,
    classifier: &Classifier,
    assigned: &HashMap<String, Option<String>>,
) -> MeetingProject {
    match assigned.get(&meeting.uid) {
        Some(Some(project)) => MeetingProject::Project(project.clone()),
        Some(None) => MeetingProject::Ignored,
        None => classifier
            .classify_parts(CALENDAR_APP_ID, &meeting.subject)
            .project
            .map_or(MeetingProject::Unassigned, MeetingProject::Project),
    }
}

type Interval = (DateTime<Utc>, DateTime<Utc>);

/// `pieces`'ten `cut` aralığını çıkarır.
fn subtract(pieces: Vec<Interval>, cut: Interval) -> Vec<Interval> {
    let mut out = Vec::with_capacity(pieces.len() + 1);
    for (a, b) in pieces {
        if cut.1 <= a || cut.0 >= b {
            out.push((a, b));
            continue;
        }
        if cut.0 > a {
            out.push((a, cut.0));
        }
        if cut.1 < b {
            out.push((cut.1, b));
        }
    }
    out
}

/// Tarayıcıda Google Meet / Teams sekmesi de toplantıdır.
const MEETING_TITLES: &[&str] = &["google meet", "meet - ", "microsoft teams", "zoom meeting"];

fn kind_of(session: &Session, config: &TimesheetConfig) -> EntryKind {
    // Elle eklenen ve projeye atanan boşta süre bilgisayar dışında geçmiştir.
    if session.is_manual() || session.is_idle() {
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

/// Açıklamada en çok bu kadar pencere başlığı.
const MAX_DETAIL_TITLES: usize = 3;
/// İlkinden sonraki başlık, kaydın en az bu oranını kaplıyorsa açıklamaya girer.
const DETAIL_SHARE: f64 = 0.2;
/// Açıklamanın en uzun hali (karakter).
const MAX_DETAILS: usize = 160;
/// Açıklama sayılmayan başlıklar (küçük harf).
const GENERIC_TITLES: &[&str] = &[
    "new tab",
    "yeni sekme",
    "untitled",
    "adsız",
    "başlıksız",
    "start page",
    "başlangıç sayfası",
    "home",
];

/// Kaydın pencere başlıklarından açıklama: başlıklarda geçen iş anahtarları (Jira gibi,
/// `ABC-123`) başta, ardından süreye göre en önemli başlıklar. Tamamen yerel, kural tabanlı:
/// tek başlık varsa açıklama odur.
pub fn describe(titles: HashMap<String, Duration>) -> String {
    let mut titles: Vec<(String, Duration)> = titles
        .into_iter()
        .filter(|(t, _)| !t.is_empty() && !GENERIC_TITLES.contains(&t.to_lowercase().as_str()))
        .collect();
    titles.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total = titles
        .iter()
        .map(|(_, d)| d.num_seconds())
        .sum::<i64>()
        .max(1) as f64;

    let mut keys: Vec<String> = Vec::new();
    for (t, _) in &titles {
        for key in issue_keys(t) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys.truncate(3);

    let mut picked: Vec<String> = Vec::new();
    for (i, (t, d)) in titles.iter().enumerate() {
        if picked.len() >= MAX_DETAIL_TITLES
            || (i > 0 && (d.num_seconds() as f64) < total * DETAIL_SHARE)
        {
            break;
        }
        let mut text = t.clone();
        for key in &keys {
            text = text.replace(key.as_str(), "");
        }
        let text = text
            .trim_matches(|c: char| {
                c.is_whitespace() || matches!(c, ':' | '-' | '|' | '·' | '[' | ']')
            })
            .to_string();
        let lower = text.to_lowercase();
        let duplicate = picked.iter().any(|p| {
            let p = p.to_lowercase();
            p.contains(&lower) || lower.contains(&p)
        });
        if !text.is_empty() && !duplicate {
            picked.push(text);
        }
    }

    let body = picked.join("; ");
    let out = match (keys.is_empty(), body.is_empty()) {
        (true, _) => body,
        (false, true) => keys.join(", "),
        (false, false) => format!("{}: {body}", keys.join(", ")),
    };
    if out.chars().count() > MAX_DETAILS {
        let cut: String = out.chars().take(MAX_DETAILS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        out
    }
}

/// Başlıktaki iş anahtarları: büyük harfle başlayan 2-10 büyük harf/rakam, tire, rakamlar
/// (`PROJ-12`, `ABC2-7`).
pub(crate) fn issue_keys(title: &str) -> Vec<String> {
    title
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter_map(|word| {
            let (prefix, number) = word.split_once('-')?;
            let ok = (2..=10).contains(&prefix.len())
                && prefix.starts_with(|c: char| c.is_ascii_uppercase())
                && prefix
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && (1..=6).contains(&number.len())
                && number.chars().all(|c| c.is_ascii_digit());
            ok.then(|| word.to_string())
        })
        .collect()
}

/// Kırpılmış oturum: (başlangıç, bitiş, oturum, proje, tür).
type Span<'a> = (DateTime<Utc>, DateTime<Utc>, &'a Session, String, EntryKind);

/// Günün oturumlarından ve toplantılarından iş kaydı önerileri; başlangıca göre sıralı.
/// `meetings` projesi belli toplantılardır ([`meeting_project`]). `day_start`/`day_end`
/// yerel günün sınırlarıdır; oturumlar ve toplantılar bunlara kırpılır.
pub fn propose(
    sessions: &[Session],
    meetings: &[(Meeting, String)],
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
    // Toplantılar: üst üste binenlerde ortak süre ilkine yazılır.
    let mut sorted: Vec<&(Meeting, String)> = meetings.iter().collect();
    sorted.sort_by_key(|(m, _)| (m.start, m.end));
    let mut covered: Vec<Interval> = Vec::new();
    let mut meeting_runs: Vec<Run> = Vec::new();
    for (m, project) in sorted {
        let span = (m.start.max(day_start), m.end.min(day_end));
        if span.1 <= span.0 {
            continue;
        }
        let pieces = covered.iter().fold(vec![span], |p, &c| subtract(p, c));
        covered.push(span);
        let Some(&(start, _)) = pieces.first() else {
            continue;
        };
        let worked = pieces
            .iter()
            .fold(Duration::zero(), |t, (a, b)| t + (*b - *a));
        meeting_runs.push(Run {
            project: project.clone(),
            kind: if m.online {
                EntryKind::Online
            } else {
                EntryKind::F2F
            },
            start,
            end: start,
            worked,
            titles: HashMap::from([(m.subject.clone(), worked)]),
        });
    }

    let mut spans: Vec<Span> = sessions
        .iter()
        .filter_map(|s| {
            let project = classifier.classify(s).project?;
            let (a, b) = (s.started_at.max(day_start), s.ended_at.min(day_end));
            (b > a).then(|| (a, b, s, project, kind_of(s, config)))
        })
        // Toplantı süresince takip edilen iş, toplantının kaydında sayılır.
        .flat_map(|(a, b, s, project, kind)| {
            covered
                .iter()
                .fold(vec![(a, b)], |p, &c| subtract(p, c))
                .into_iter()
                .map(move |(a, b)| (a, b, s, project.clone(), kind))
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
    let meetings_from = runs.len();
    runs.extend(meeting_runs);

    let mut out: Vec<TimesheetEntry> = runs
        .into_iter()
        .enumerate()
        .filter(|(_, r)| r.worked >= MIN_ENTRY)
        .map(|(i, r)| (i >= meetings_from, r))
        .map(|(is_meeting, r)| {
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
            let details = if r.kind == EntryKind::Online && !is_meeting {
                // Toplantı uygulamasının başlığı ("Zoom Meeting") açıklama değildir;
                // takvimden gelen toplantının konusu ise açıklamadır.
                String::new()
            } else {
                describe(r.titles)
            };
            // Hazır açıklama yalnızca boş kalan açıklamayı doldurur: başlıklardan çıkan metin
            // (iş anahtarı, belge adı) projenin genel metninden daha bilgilendiricidir; boş
            // satır ise aktarılamaz. Doldurulan metin elle değiştirilebilir.
            let details = match mapping.and_then(|m| m.default_details.as_deref()) {
                Some(text) if details.trim().is_empty() => text.trim().to_string(),
                _ => details,
            };
            let local = r.start.with_timezone(&Local);
            TimesheetEntry {
                date: local.date_naive(),
                start: local.time().with_nanosecond(0).unwrap_or(local.time()),
                hours: round_quarter(r.worked.num_seconds() as f64 / 3600.0),
                actual_hours: Some(r.worked.num_seconds() as f64 / 3600.0),
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

/// Yeniden öneride Excel'e aktarılmış işi düşer: her (proje, tür) için aktarılan gerçek süre
/// (yoksa yazılan saat) o türün en erken önerilerinden düşülür, yalnızca artan süre kalır
/// (aktarılan + önerilen = takip edilen); kalanın saati yeniden yuvarlanır. Aktarılan satırın saati ya
/// da süresi elle değiştirilmiş olsa da iş ikinci kez önerilmez; aktarımdan sonra süren iş
/// (uzayan kayıt ya da yeni kayıt) önerilir. Kısmen düşülen önerinin kalanı `MIN_REMAINDER`'dan
/// kısaysa atılır, değilse başlangıcı düşülen süre kadar ileri alınır.
pub fn without_exported(
    proposed: &[TimesheetEntry],
    exported: &[TimesheetEntry],
) -> Vec<TimesheetEntry> {
    let min_rest = MIN_REMAINDER.num_seconds() as f64 / 3600.0;
    let mut budget: HashMap<(&str, EntryKind), f64> = HashMap::new();
    for x in exported {
        *budget.entry((x.project_id.as_str(), x.kind)).or_default() += x.worked();
    }
    let mut sorted: Vec<&TimesheetEntry> = proposed.iter().collect();
    sorted.sort_by_key(|e| e.start);
    let mut out = Vec::new();
    for e in sorted {
        let left = budget.entry((e.project_id.as_str(), e.kind)).or_default();
        let used = left.min(e.worked());
        *left -= used;
        let rest = e.worked() - used;
        if used == 0.0 {
            out.push(e.clone());
        } else if rest >= min_rest {
            let shift = Duration::seconds((used * 3600.0).round() as i64);
            out.push(TimesheetEntry {
                start: e.start.overflowing_add_signed(shift).0,
                hours: round_quarter(rest),
                actual_hours: Some(rest),
                ..e.clone()
            });
        }
    }
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
                default_details: None,
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
        let got = propose(
            &sessions,
            &[],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        let rows: Vec<_> = got
            .iter()
            .map(|e| {
                (
                    e.start.format("%H:%M").to_string(),
                    (e.worked() * 60.0).round() as i64,
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
    fn details_summarize_issue_keys_and_main_titles() {
        let m = |n: i64| Duration::minutes(n);
        let one = HashMap::from([("Trumore Loyalty UI/UX".to_string(), m(40))]);
        assert_eq!(describe(one), "Trumore Loyalty UI/UX");
        let many = HashMap::from([
            ("PROJ-12 Ödeme ekranı hatası - Jira".to_string(), m(30)),
            ("[PROJ-15] Sepet tutarı".to_string(), m(20)),
            ("Yeni Sekme".to_string(), m(15)),
            ("main.rs — kum".to_string(), m(25)),
            ("Haberler".to_string(), m(2)),
        ]);
        assert_eq!(
            describe(many),
            "PROJ-12, PROJ-15: Ödeme ekranı hatası - Jira; main.rs — kum; Sepet tutarı"
        );
        assert_eq!(
            describe(HashMap::from([("ABC-1".to_string(), m(5))])),
            "ABC-1"
        );
        assert_eq!(
            describe(HashMap::from([("Yeni Sekme".to_string(), m(5))])),
            ""
        );
        let long = HashMap::from([("x".repeat(400), m(5))]);
        assert_eq!(describe(long).chars().count(), MAX_DETAILS);
        // "COVID-19" gibi sözcükler de anahtara benzer; kabul edilebilir, ama küçük harfli değil.
        assert!(issue_keys("covid-19 ve utf-8").is_empty());
    }

    #[test]
    fn manual_entries_are_face_to_face_and_mapping_applies() {
        let (classifier, names, mut config) = setup();
        config.projects[0].party = Some("Togg".into());
        let mut meeting = s("kum.manual/Workshop", "Workshop", 0, 60, Some("sync"));
        meeting.app_name = "Workshop".into();
        let got = propose(
            &[meeting],
            &[],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, EntryKind::F2F);
        assert_eq!(got[0].details, "Workshop");
        assert_eq!(
            (got[0].division.as_str(), got[0].party.as_str()),
            ("Int.Work.Sync.", "Togg")
        );
        assert!((got[0].hours - 1.0).abs() < 1e-9);
        assert_eq!(got[0].actual_hours, Some(1.0));
    }

    #[test]
    fn default_details_fill_only_empty_descriptions() {
        let (classifier, names, mut config) = setup();
        config.projects.push(ProjectMapping {
            project_id: "tru".into(),
            division: "Trumore".into(),
            party: None,
            default_details: Some("  Trumore danışmanlık ".into()),
        });
        let sessions = [
            // Başlıktan açıklama çıkar: hazır metin kullanılmaz.
            s("Figma", "Trumore Pitchdeck — Figma", 0, 30, None),
            // Toplantı uygulaması: açıklama boş kalır, hazır metin girer.
            s("us.zoom.xos", "Zoom Meeting", 30, 60, Some("tru")),
            // Hazır metni olmayan projenin boş açıklaması boş kalır.
            s("us.zoom.xos", "Zoom Meeting", 60, 90, Some("sync")),
        ];
        let got = propose(
            &sessions,
            &[],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        let details: Vec<(&str, &str)> = got
            .iter()
            .map(|e| (e.project_id.as_str(), e.details.as_str()))
            .collect();
        assert_eq!(
            details,
            [
                ("tru", "Trumore Pitchdeck"),
                ("tru", "Trumore danışmanlık"),
                ("sync", "")
            ]
        );
    }

    #[test]
    fn mapping_without_default_details_still_parses() {
        let m: ProjectMapping =
            serde_json::from_str(r#"{"projectId":"a","division":"A","party":null}"#).unwrap();
        assert_eq!(m.default_details, None);
    }

    #[test]
    fn assigned_idle_time_is_face_to_face() {
        let (c, names, config) = setup();
        let mut away = Session::idle(t(0), t(60));
        away.project_id = Some("tru".into());
        let out = propose(&[away], &[], &c, &names, &config, t(-600), t(600));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EntryKind::F2F);
        assert_eq!(out[0].hours, 1.0);
    }

    #[test]
    fn browser_meetings_are_online() {
        let (classifier, names, config) = setup();
        let got = propose(
            &[s("com.google.Chrome", "Meet - Trumore weekly", 0, 30, None)],
            &[],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        assert_eq!(got[0].kind, EntryKind::Online);
    }

    fn meeting(uid: &str, subject: &str, from: i64, to: i64, online: bool) -> Meeting {
        Meeting {
            uid: uid.into(),
            start: t(from),
            end: t(to),
            subject: subject.into(),
            location: String::new(),
            online,
            ..Meeting::default()
        }
    }

    #[test]
    fn calendar_meetings_become_entries_and_replace_tracked_time() {
        let (classifier, names, config) = setup();
        let sessions = [
            // 09:00–10:30 Figma; 09:30–10:00 arası toplantıdaydı (ekran paylaşımı).
            s("Figma", "Trumore Loyalty UI/UX — Figma", 0, 90, None),
        ];
        let meetings = [
            (
                meeting("w", "Trumore haftalık", 30, 60, true),
                "tru".to_string(),
            ),
            // Yüz yüze; ilk 15 dakikası öncekiyle çakışıyor (iki kez sayılmaz).
            (
                meeting("f", "Sync atölye", 45, 105, false),
                "sync".to_string(),
            ),
        ];
        let got = propose(
            &sessions,
            &meetings,
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        let rows: Vec<_> = got
            .iter()
            .map(|e| {
                (
                    e.start.format("%H:%M").to_string(),
                    (e.worked() * 60.0).round() as i64,
                    e.kind,
                    e.details.as_str(),
                    e.division.as_str(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                // Figma 90 dk − toplantılar (09:30–10:30 kapalı) = 30 dk.
                (
                    "09:00".to_string(),
                    30,
                    EntryKind::Working,
                    "Trumore Loyalty UI/UX",
                    "Trumore"
                ),
                (
                    "09:30".to_string(),
                    30,
                    EntryKind::Online,
                    "Trumore haftalık",
                    "Trumore"
                ),
                (
                    "10:00".to_string(),
                    45,
                    EntryKind::F2F,
                    "Sync atölye",
                    "Int.Work.Sync."
                ),
            ]
        );
    }

    #[test]
    fn meeting_project_prefers_assignment_then_rules() {
        let (classifier, _, _) = setup();
        let m = meeting("seri", "Trumore weekly", 0, 30, true);
        let mut assigned = HashMap::new();
        assert_eq!(
            meeting_project(&m, &classifier, &assigned),
            MeetingProject::Project("tru".into())
        );
        let other = meeting("x", "1:1", 0, 30, true);
        assert_eq!(
            meeting_project(&other, &classifier, &assigned),
            MeetingProject::Unassigned
        );
        assigned.insert("seri".to_string(), Some("sync".to_string()));
        assigned.insert("x".to_string(), None);
        assert_eq!(
            meeting_project(&m, &classifier, &assigned),
            MeetingProject::Project("sync".into())
        );
        assert_eq!(
            meeting_project(&other, &classifier, &assigned),
            MeetingProject::Ignored
        );
    }

    fn entry(project: &str, kind: EntryKind, hh: u32, mm: u32, hours: f64) -> TimesheetEntry {
        TimesheetEntry {
            date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            start: NaiveTime::from_hms_opt(hh, mm, 0).unwrap(),
            hours,
            actual_hours: None,
            kind,
            details: String::new(),
            party: String::new(),
            project_id: project.into(),
            division: project.into(),
        }
    }

    #[test]
    fn hours_are_rounded_to_quarters_and_actual_is_kept() {
        let (classifier, names, config) = setup();
        let sessions = [
            s("Figma", "Trumore — Figma", 0, 67, None), // 1 sa 7 dk → 1,00
            s("Figma", "Trumore — Figma", 120, 128, None), // 8 dk → 0,25 (en az)
            s("Figma", "Trumore — Figma", 200, 253, None), // 53 dk → 1,00 (0,88 → 1)
        ];
        let got = propose(
            &sessions,
            &[],
            &classifier,
            &names,
            &config,
            t(-540),
            t(900),
        );
        let hours: Vec<(f64, i64)> = got
            .iter()
            .map(|e| (e.hours, (e.worked() * 60.0).round() as i64))
            .collect();
        assert_eq!(hours, [(1.0, 67), (0.25, 8), (1.0, 53)]);
        assert_eq!(round_quarter(0.37), 0.25);
        assert_eq!(round_quarter(0.38), 0.5);
        assert_eq!(round_quarter(2.6), 2.5);
    }

    #[test]
    fn exported_work_is_not_proposed_again() {
        use EntryKind::{Online, Working};
        let short = |v: Vec<TimesheetEntry>| -> Vec<(String, String, f64)> {
            v.into_iter()
                .map(|e| {
                    let worked = (e.worked() * 100.0).round() / 100.0;
                    (e.project_id, e.start.format("%H:%M").to_string(), worked)
                })
                .collect()
        };
        let proposed = [
            entry("a", Working, 9, 10, 0.83),
            entry("a", Working, 13, 0, 2.0),
            entry("a", Online, 11, 0, 0.5),
            entry("b", Working, 10, 0, 1.0),
        ];
        // Hiç aktarım yoksa öneriler aynen.
        assert_eq!(without_exported(&proposed, &[]).len(), 4);
        // Aktarırken 09:10 kaydının saati 09:00'a çekilmiş ve 0,75'e yuvarlanmış; 13:00 kaydı
        // aktarıldığında 1 saatti, sonra 2 saate uzadı. Düzenlenmiş kayıt tekrar önerilmez;
        // toplam korunur (aktarılan 1,75 + önerilen 1,08 = takip edilen 2,83). Toplantı ve
        // b projesi aktarılmadı: aynen kalır.
        let exported = [
            entry("a", Working, 9, 0, 0.75),
            entry("a", Working, 13, 0, 1.0),
        ];
        assert_eq!(
            short(without_exported(&proposed, &exported)),
            [
                ("b", "10:00", 1.0),
                ("a", "11:00", 0.5),
                ("a", "13:55", 1.08)
            ]
            .map(|(p, t, h)| (p.to_string(), t.to_string(), h))
        );
        // Kalan 15 dakikadan kısaysa (yuvarlama artığı) önerilmez.
        let exported = [entry("a", Working, 9, 0, 2.75)];
        assert!(
            without_exported(&proposed, &exported)
                .iter()
                .all(|e| e.project_id != "a" || e.kind != Working)
        );
        // Tamamı aktarılmış gün: yeni bir şey yok.
        assert!(without_exported(&proposed, &proposed).is_empty());
    }
}
