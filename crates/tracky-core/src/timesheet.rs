//! Zaman çizelgesi: günün projeye atanmış süresini firmaya gönderilecek iş kayıtlarına
//! (tarih, başlangıç, saat, tür, açıklama, taraf, birim) dönüştürür.
//!
//! Her zaman çizelgesi ([`Timesheet`]) bir firmanın dosyasıdır (Excel ya da Google Sheets) ve
//! yalnızca kendisine bağlanan projelerin işini alır: Togg'un tablosuna yalnızca Togg'a bağlı
//! projelerin süresi yazılır. Bir proje en çok bir zaman çizelgesine bağlanır.
//!
//! Yalnızca bir projeye düşen süre iş sayılır. Aynı projede ve aynı türdeki ardışık
//! oturumlar, aradaki boşluk `MERGE_GAP`'i geçmedikçe tek kayıt olur; kaydın gerçek süresi
//! boşluklar değil, oturumların toplam süresidir. Firmaya giden saat bu sürenin çeyrek saate
//! yuvarlanmışıdır ([`round_quarter`]); gerçek süre kayıtta ayrıca saklanır.
//!
//! Takvimden gelen ve bir projeye düşen toplantılar kendi kaydı olur (konusu açıklama,
//! çevrim içiyse Online, değilse F2F). Toplantı süresince takip edilen iş sayılmaz: aynı
//! saat iki kez yazılmasın.
//!
//! Öneriler canlıdır: raporda projeye atanan süre hemen satır olur. Düzenlenen, birleştirilen,
//! gizlenen ya da aktarılan satır kaydedilir ve kapsadığı takip aralıklarını
//! ([`TimesheetEntry::coverage`]) saklar; bu aralıklar yeni önerilerden düşülür ([`propose`]).
//! Sonradan atanan iş kaydedilmiş satırlara dokunmadan yeni satır olarak gelir, aynı iş iki kez
//! yazılmaz.
//!
//! Firmanın dosyasındaki satırlar ([`FileRow`]) da okunur: Kum'un aktardığı satırlar dosyadaki
//! satırlarıyla eşlenir ([`link_file_rows`]), Kum dışında girilen satırlar ayrıca gösterilir.
//! Aktarılmış satır düzenlenince değişiklik dosyadaki satırına da yazılır.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::Classifier;
use crate::model::Session;

/// Aynı proje ve türdeki iki oturum arasında bundan kısa boşluk kaydı bölmez.
pub const MERGE_GAP: Duration = Duration::minutes(15);
/// Bundan kısa kayıt önerilmez (bir mesaja bakmak gibi kısa geçişler).
pub const MIN_ENTRY: Duration = Duration::minutes(5);
/// Kaydedilmiş bir satırın hemen ardından (ya da önünden) kalan süre bundan kısaysa önerilmez:
/// satır kaydedilirken süren işin ya da aktarırken saati yuvarlamaktan (0,83 → 0,75) kalan birkaç
/// dakika yeni iş değildir.
pub const MIN_REMAINDER: Duration = Duration::minutes(15);
/// Kaydedilmiş satırın aralıklarında projenin süresi bundan fazla azaldıysa satır "takipte
/// değişti" sayılır (iş raporda başka projeye ya da projesize alınmış).
pub const STALE_SLACK: Duration = Duration::minutes(1);

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

/// Zaman çizelgesi ayarları (cihazlar arasında eşitlenir; bkz. [`crate::sync::SYNCED_SETTINGS`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimesheetConfig {
    /// Firmaların zaman çizelgeleri; her biri yalnızca kendi projelerinin işini alır.
    pub timesheets: Vec<Timesheet>,
    /// Apps Script'in yalnızca Kum'dan gelen istekleri kabul etmesi için anahtar (bütün
    /// tablolarda aynı).
    pub sheet_token: String,
    /// Çevrim içi toplantı sayılan uygulamalar (kimlik ya da exe adı, `*` öneki olabilir).
    pub meeting_apps: Vec<String>,
    /// Bir adam-günün saati (adam-gün = saat / bu değer).
    pub day_hours: f64,
}

/// Bir firmanın zaman çizelgesi: kayıtların yazıldığı dosya ve oraya giden projeler.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Timesheet {
    pub id: String,
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
    /// "Parties" varsayılanı (örn. kendi firman).
    pub default_party: String,
    /// Bu çizelgeye giden projeler; birim ("Togg Division"), taraf ve hazır açıklamalarıyla.
    pub projects: Vec<ProjectMapping>,
    /// Dosyadaki birimler (şablondan): satırın birimi bunlardan seçilebilir.
    pub divisions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMapping {
    pub project_id: String,
    /// Projenin satırlarına yazılan birim; boşsa proje adı kullanılır.
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
            timesheets: Vec::new(),
            sheet_token: String::new(),
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

impl TimesheetConfig {
    pub fn timesheet(&self, id: &str) -> Option<&Timesheet> {
        self.timesheets.iter().find(|t| t.id == id)
    }

    /// Projenin bağlı olduğu zaman çizelgesi.
    pub fn timesheet_of(&self, project: &str) -> Option<&Timesheet> {
        self.timesheets.iter().find(|t| t.includes(project))
    }

    /// Kaydetmeden önce: her çizelgenin (tekil) kimliği olur, bir proje yalnızca ilk
    /// çizelgesinde kalır, birimler kırpılır ve tekrarlanmaz.
    pub fn normalize(&mut self) {
        let mut ids = HashSet::new();
        let mut projects = HashSet::new();
        for sheet in &mut self.timesheets {
            if sheet.id.trim().is_empty() || !ids.insert(sheet.id.clone()) {
                sheet.id = uuid::Uuid::new_v4().to_string();
                ids.insert(sheet.id.clone());
            }
            sheet
                .projects
                .retain(|m| projects.insert(m.project_id.clone()));
            let mut divisions: Vec<String> = Vec::new();
            for d in sheet.divisions.drain(..) {
                let d = d.trim();
                if !d.is_empty() && !divisions.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                    divisions.push(d.to_string());
                }
            }
            sheet.divisions = divisions;
        }
    }
}

impl Timesheet {
    /// Kayıtların yazılacağı dosya ya da tablo seçili.
    pub fn has_target(&self) -> bool {
        self.file_path.is_some() || self.sheet_url.is_some()
    }

    /// Proje bu çizelgeye gidiyor mu?
    pub fn includes(&self, project: &str) -> bool {
        self.projects.iter().any(|m| m.project_id == project)
    }

    pub fn mapping(&self, project: &str) -> Option<&ProjectMapping> {
        self.projects.iter().find(|m| m.project_id == project)
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
    /// Satırın kapsadığı takip aralıkları (unix ms, `[başlangıç, bitiş)`, sıralı ve ayrık); bu
    /// süre yeniden önerilmez. Boş: elle eklenen satır. `None`: önceki sürümde kaydedilmiş,
    /// aralıkları bilinmeyen satır; işi saatiyle düşülür ([`without_legacy`]).
    #[serde(default)]
    pub coverage: Option<Vec<[i64; 2]>>,
}

impl TimesheetEntry {
    /// Gerçek süre; bilinmiyorsa yazılan saat.
    pub fn worked(&self) -> f64 {
        self.actual_hours.unwrap_or(self.hours)
    }

    /// Takip edilen aralıklar; elle eklenen ve eski satırda boş.
    pub fn spans(&self) -> Vec<Interval> {
        self.coverage
            .as_deref()
            .map(from_coverage)
            .unwrap_or_default()
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

pub type Interval = (DateTime<Utc>, DateTime<Utc>);

/// `pieces`'ten `cut` aralığını çıkarır.
pub(crate) fn subtract(pieces: Vec<Interval>, cut: Interval) -> Vec<Interval> {
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

/// Aralıkları sıralar; üst üste binen ya da bitişik olanları birleştirir, boşları atar.
pub fn coalesce(mut spans: Vec<Interval>) -> Vec<Interval> {
    spans.retain(|(a, b)| b > a);
    spans.sort();
    let mut out: Vec<Interval> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Aralıklar → satırda saklanan biçim (unix ms).
pub fn to_coverage(spans: &[Interval]) -> Vec<[i64; 2]> {
    spans
        .iter()
        .map(|(a, b)| [a.timestamp_millis(), b.timestamp_millis()])
        .collect()
}

/// Satırda saklanan biçim → aralıklar.
pub fn from_coverage(coverage: &[[i64; 2]]) -> Vec<Interval> {
    let at = |ms: i64| Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
    coverage.iter().map(|[a, b]| (at(*a), at(*b))).collect()
}

/// Aralıkların toplam süresi.
pub fn total(spans: &[Interval]) -> Duration {
    spans.iter().fold(Duration::zero(), |t, (a, b)| {
        t + (*b - *a).max(Duration::zero())
    })
}

/// `(a, b)` ile `spans`'in kesişimi (`spans` ayrık olmalı).
fn intersect(a: DateTime<Utc>, b: DateTime<Utc>, spans: &[Interval]) -> Vec<Interval> {
    spans
        .iter()
        .map(|&(c, d)| (a.max(c), b.min(d)))
        .filter(|(x, y)| y > x)
        .collect()
}

/// Tarayıcıda Google Meet / Teams sekmesi de toplantıdır.
const MEETING_TITLES: &[&str] = &["google meet", "meet - ", "microsoft teams", "zoom meeting"];

pub(crate) fn kind_of(session: &Session, config: &TimesheetConfig) -> EntryKind {
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
pub(crate) fn clean_title(title: &str, app_name: &str) -> String {
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
pub(crate) const GENERIC_TITLES: &[&str] = &[
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

/// Zaman çizelgesine giren süre parçası: projesi belli bir oturumdan ya da toplantıdan.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub project: String,
    pub kind: EntryKind,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Açıklamaya giden başlık: temizlenmiş pencere başlığı ya da toplantı konusu.
    pub title: String,
    /// Takvim toplantısının sırası (her toplantı kendi kaydıdır); oturumda `None`.
    pub meeting: Option<usize>,
}

/// Günün zaman çizelgesine giren süresi: projesi belli toplantılar ([`meeting_project`]) ve
/// oturumlar, `day_start`–`day_end` (yerel gün) sınırına kırpılmış. Üst üste binen
/// toplantılarda ortak süre ilkine yazılır; toplantı süresince takip edilen iş toplantının
/// kaydında sayılır (aynı saat iki kez yazılmasın).
pub fn pieces(
    sessions: &[Session],
    meetings: &[(Meeting, String)],
    classifier: &Classifier,
    config: &TimesheetConfig,
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Vec<Piece> {
    let mut sorted: Vec<&(Meeting, String)> = meetings.iter().collect();
    sorted.sort_by_key(|(m, _)| (m.start, m.end));
    let mut covered: Vec<Interval> = Vec::new();
    let mut out = Vec::new();
    for (i, (m, project)) in sorted.into_iter().enumerate() {
        let span = (m.start.max(day_start), m.end.min(day_end));
        if span.1 <= span.0 {
            continue;
        }
        let parts = covered.iter().fold(vec![span], |p, &c| subtract(p, c));
        covered.push(span);
        let kind = if m.online {
            EntryKind::Online
        } else {
            EntryKind::F2F
        };
        out.extend(parts.into_iter().map(|(start, end)| Piece {
            project: project.clone(),
            kind,
            start,
            end,
            title: m.subject.clone(),
            meeting: Some(i),
        }));
    }
    for s in sessions {
        let Some(project) = classifier.classify(s).project else {
            continue;
        };
        let (a, b) = (s.started_at.max(day_start), s.ended_at.min(day_end));
        if b <= a {
            continue;
        }
        let kind = kind_of(s, config);
        let title = clean_title(&s.title, &s.app_name);
        let parts = covered.iter().fold(vec![(a, b)], |p, &c| subtract(p, c));
        out.extend(parts.into_iter().map(|(start, end)| Piece {
            project: project.clone(),
            kind,
            start,
            end,
            title: title.clone(),
            meeting: None,
        }));
    }
    out
}

/// Parçalardan `sheet`'e bağlı projelerin iş kaydı önerileri; başlangıca göre sıralı.
/// `saved` proje başına kaydedilmiş satırların aralıklarıdır ([`coalesce`] edilmiş): bu süre
/// yeniden önerilmez. Kaydedilmiş satırın artığı `MIN_REMAINDER`'dan kısaysa önerilmez: satır
/// kaydedilirken süren işin birkaç dakikası (satıra `MERGE_GAP`'ten yakın oturumlar) ya da bir
/// kısmı kaydedilmiş toplantının uzayan ucu yeni iş sayılmaz. Kaydedilmiş toplantının ardından
/// gelen ayrı bir toplantı ise kısa da olsa önerilir.
pub fn propose(
    pieces: &[Piece],
    project_names: &HashMap<String, String>,
    sheet: &Timesheet,
    saved: &HashMap<String, Vec<Interval>>,
) -> Vec<TimesheetEntry> {
    struct Run {
        project: String,
        kind: EntryKind,
        meeting: bool,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        worked: Duration,
        titles: HashMap<String, Duration>,
        spans: Vec<Interval>,
        /// Parçalarından biri kaydedilmiş bir satırca kısaltıldı.
        trimmed: bool,
    }
    impl Run {
        fn new(p: &Piece, at: DateTime<Utc>) -> Self {
            Self {
                project: p.project.clone(),
                kind: p.kind,
                meeting: p.meeting.is_some(),
                start: at,
                end: at,
                worked: Duration::zero(),
                titles: HashMap::new(),
                spans: Vec::new(),
                trimmed: false,
            }
        }

        fn add(&mut self, a: DateTime<Utc>, b: DateTime<Utc>, title: &str, trimmed: bool) {
            self.trimmed |= trimmed;
            self.end = self.end.max(b);
            self.worked += b - a;
            *self
                .titles
                .entry(title.to_string())
                .or_insert(Duration::zero()) += b - a;
            self.spans.push((a, b));
        }
    }

    let none: Vec<Interval> = Vec::new();
    let mut parts: Vec<(DateTime<Utc>, DateTime<Utc>, &Piece, bool)> = pieces
        .iter()
        .filter(|p| sheet.includes(&p.project))
        .flat_map(|p| {
            let left = saved
                .get(&p.project)
                .unwrap_or(&none)
                .iter()
                .fold(vec![(p.start, p.end)], |v, &c| subtract(v, c));
            let trimmed = total(&left) < p.end - p.start;
            left.into_iter().map(move |(a, b)| (a, b, p, trimmed))
        })
        .collect();
    parts.sort_by_key(|(a, _, _, _)| *a);

    // Her toplantı kendi kaydı; oturumlarda her (proje, tür) için açık kayıt, araya başka iş
    // girse de boşluk kısaysa sürer.
    let mut meetings: Vec<(usize, Run)> = Vec::new();
    let mut open: HashMap<(String, EntryKind), Run> = HashMap::new();
    let mut runs: Vec<Run> = Vec::new();
    for (a, b, p, trimmed) in parts {
        if let Some(i) = p.meeting {
            match meetings.iter_mut().find(|(j, _)| *j == i) {
                Some((_, run)) => run.add(a, b, &p.title, trimmed),
                None => {
                    let mut run = Run::new(p, a);
                    run.add(a, b, &p.title, trimmed);
                    meetings.push((i, run));
                }
            }
            continue;
        }
        let key = (p.project.clone(), p.kind);
        if let Some(run) = open.get(&key)
            && a - run.end > MERGE_GAP
        {
            runs.push(open.remove(&key).expect("az önce bulundu"));
        }
        open.entry(key)
            .or_insert_with(|| Run::new(p, a))
            .add(a, b, &p.title, trimmed);
    }
    runs.extend(open.into_values());
    runs.extend(meetings.into_iter().map(|(_, run)| run));

    // Kaydedilmiş satırın artığı mı: oturumlarda satıra yakın iş, toplantıda aynı toplantının ucu.
    let remainder = |r: &Run| {
        if r.meeting {
            return r.trimmed;
        }
        saved.get(&r.project).is_some_and(|cut| {
            r.spans.iter().any(|&(a, b)| {
                cut.iter()
                    .any(|&(c, d)| a <= d + MERGE_GAP && c <= b + MERGE_GAP)
            })
        })
    };
    let mut out: Vec<TimesheetEntry> = runs
        .into_iter()
        .filter(|r| r.worked >= MIN_ENTRY && (r.worked >= MIN_REMAINDER || !remainder(r)))
        .map(|r| {
            let mapping = sheet.mapping(&r.project);
            let name = project_names.get(&r.project).cloned().unwrap_or_default();
            let division = mapping
                .map(|m| m.division.trim())
                .filter(|d| !d.is_empty())
                .map_or(name, str::to_string);
            let party = mapping
                .and_then(|m| m.party.clone())
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| sheet.default_party.clone());
            let details = if r.kind == EntryKind::Online && !r.meeting {
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
            let worked = r.worked.num_seconds() as f64 / 3600.0;
            TimesheetEntry {
                date: local.date_naive(),
                start: local.time().with_nanosecond(0).unwrap_or(local.time()),
                hours: round_quarter(worked),
                actual_hours: Some(worked),
                kind: r.kind,
                details,
                party,
                project_id: r.project,
                division,
                coverage: Some(to_coverage(&coalesce(r.spans))),
            }
        })
        .collect();
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.division.cmp(&b.division)));
    out
}

/// Aralıkları bilinmeyen eski satırların (önceki sürümde onaylanan ya da aktarılan) işini
/// önerilerden düşer: her (proje, tür) için eski satırların gerçek süresi (yoksa yazılan saat)
/// o türün en erken önerilerinden düşülür, yalnızca artan süre kalır (eski + önerilen = takip
/// edilen); kalanın saati yeniden yuvarlanır. Satırın saati ya da süresi elle değiştirilmiş olsa
/// da iş ikinci kez önerilmez. Kısmen düşülen önerinin kalanı `MIN_REMAINDER`'dan kısaysa atılır,
/// değilse başlangıcı ve aralıkları düşülen süre kadar ileri alınır.
pub fn without_legacy(
    proposed: &[TimesheetEntry],
    legacy: &[TimesheetEntry],
) -> Vec<TimesheetEntry> {
    let min_rest = MIN_REMAINDER.num_seconds() as f64 / 3600.0;
    let mut budget: HashMap<(&str, EntryKind), f64> = HashMap::new();
    for x in legacy {
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
                coverage: e
                    .coverage
                    .as_deref()
                    .map(|c| to_coverage(&skip_front(&from_coverage(c), shift))),
                ..e.clone()
            });
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.division.cmp(&b.division)));
    out
}

/// Aralıkların ilk `skip` kadarını atar.
fn skip_front(spans: &[Interval], mut skip: Duration) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(a, b) in spans {
        let len = b - a;
        if skip >= len {
            skip -= len;
        } else {
            out.push((a + skip, b));
            skip = Duration::zero();
        }
    }
    out
}

/// `spans` aralıklarında projenin bugün hâlâ zaman çizelgesine giren süresi ve aralıkları.
/// Kaydedilmiş satırın aralıklarındaki süre azaldıysa iş raporda başka projeye ya da projesize
/// alınmış (ya da toplantı değişmiş) demektir.
pub fn still_covered(pieces: &[Piece], project: &str, spans: &[Interval]) -> Vec<Interval> {
    let spans = coalesce(spans.to_vec());
    coalesce(
        pieces
            .iter()
            .filter(|p| p.project == project)
            .flat_map(|p| intersect(p.start, p.end, &spans))
            .collect(),
    )
}

/// Kaydedilmiş satır takipte değiştiyse projede kalan süre (saat): satırın aralıklarındaki
/// süre `STALE_SLACK`'ten fazla azalmış. Elle eklenen ve eski satırlar değişmez.
pub fn stale_hours(pieces: &[Piece], entry: &TimesheetEntry) -> Option<f64> {
    let spans = entry.spans();
    if spans.is_empty() {
        return None;
    }
    let left = total(&still_covered(pieces, &entry.project_id, &spans));
    let hours = left.num_seconds() as f64 / 3600.0;
    (total(&spans) - left > STALE_SLACK).then_some(hours)
}

/// Takipte değişen satırın yeni hali: aralıkları projede kalan süreye iner, gerçek süre ve
/// (yuvarlanmış) saat ondan hesaplanır; başlangıç elle değiştirilmediyse ilk kalan ana kayar.
/// Projede hiç süre kalmadıysa `None`.
pub fn refreshed(pieces: &[Piece], entry: &TimesheetEntry) -> Option<TimesheetEntry> {
    let spans = entry.spans();
    let left = still_covered(pieces, &entry.project_id, &spans);
    let first = *left.first()?;
    let worked = total(&left).num_seconds() as f64 / 3600.0;
    let start = match spans.first() {
        Some((was, _)) if minute(was.with_timezone(&Local).time()) == minute(entry.start) => {
            first.0.with_timezone(&Local).time().with_nanosecond(0)?
        }
        _ => entry.start,
    };
    Some(TimesheetEntry {
        start,
        hours: round_quarter(worked),
        actual_hours: Some(worked),
        coverage: Some(to_coverage(&left)),
        ..entry.clone()
    })
}

fn minute(t: NaiveTime) -> (u32, u32) {
    (t.hour(), t.minute())
}

/// Firmanın dosyasındaki (Excel ya da Sheets) bir kayıt satırı: Kum'un aktardığı ya da elle
/// girilmiş.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    /// Dosyadaki satır numarası (yazarken ipucu; satır kaymışsa içeriğinden bulunur).
    pub row: u32,
    pub date: NaiveDate,
    pub start: Option<NaiveTime>,
    pub hours: Option<f64>,
    pub kind: String,
    pub details: String,
    pub party: String,
    pub division: String,
    pub consultant: String,
}

impl FileRow {
    /// Kum'un satırının dosyadaki hali (`row`: bilinen satır numarası ya da 0).
    pub fn of(e: &TimesheetEntry, row: u32) -> Self {
        Self {
            row,
            date: e.date,
            start: Some(e.start),
            hours: Some(e.hours),
            kind: e.kind.label().to_string(),
            details: e.details.trim().to_string(),
            party: e.party.trim().to_string(),
            division: e.division.trim().to_string(),
            consultant: String::new(),
        }
    }

    fn same_start(&self, e: &TimesheetEntry) -> bool {
        self.date == e.date && self.start.map(minute) == Some(minute(e.start))
    }

    /// Kum'un satırıyla birebir aynı (dosyaya yazıldığı gibi duruyor).
    pub fn matches(&self, e: &TimesheetEntry) -> bool {
        self.same_start(e)
            && self.hours.is_some_and(|h| (h - e.hours).abs() < 1e-6)
            && self.kind.trim() == e.kind.label()
            && self.details.trim() == e.details.trim()
            && self.party.trim() == e.party.trim()
            && self.division.trim().eq_ignore_ascii_case(e.division.trim())
    }

    /// Kum'un satırı dosyada değiştirilmiş hali olabilir: aynı gün ve başlangıç, tür ya da
    /// açıklama aynı.
    fn near(&self, e: &TimesheetEntry) -> bool {
        self.same_start(e)
            && (self.kind.trim() == e.kind.label() || self.details.trim() == e.details.trim())
    }

    /// Kum'un satırı dosyadaki değerleriyle (tür tanınmıyorsa ya da başlangıç ya da saat boşsa
    /// `None`). Proje, gerçek süre ve aralıklar Kum'da kalır.
    pub fn apply(&self, e: &TimesheetEntry) -> Option<TimesheetEntry> {
        let kind = match self.kind.trim() {
            "Working" => EntryKind::Working,
            "Online" => EntryKind::Online,
            "F2F" => EntryKind::F2F,
            _ => return None,
        };
        Some(TimesheetEntry {
            date: self.date,
            start: self.start?,
            hours: self.hours.filter(|h| *h > 0.0 && *h <= 24.0)?,
            kind,
            details: self.details.trim().to_string(),
            party: self.party.trim().to_string(),
            division: self.division.trim().to_string(),
            ..e.clone()
        })
    }
}

/// Dosyanın satırlarını Kum'un aktardığı satırlarla eşler: her dosya satırı için eşlendiği
/// Kum satırının sırası. Önce birebir aynı olanlar, sonra dosyada değiştirilmiş olabilecekler
/// ([`FileRow::near`]); bir satır en çok bir satırla eşlenir. Eşlenmeyen dosya satırı Kum dışında
/// girilmiştir; eşlenmeyen Kum satırı dosyada silinmiş ya da başlangıcı değiştirilmiştir.
pub fn link_file_rows(rows: &[FileRow], entries: &[&TimesheetEntry]) -> Vec<Option<usize>> {
    let mut out = vec![None; rows.len()];
    let mut used = vec![false; entries.len()];
    let passes: [fn(&FileRow, &TimesheetEntry) -> bool; 2] = [FileRow::matches, FileRow::near];
    for pass in passes {
        for (i, row) in rows.iter().enumerate() {
            if out[i].is_some() {
                continue;
            }
            if let Some(j) = (0..entries.len()).find(|&j| !used[j] && pass(row, entries[j])) {
                used[j] = true;
                out[i] = Some(j);
            }
        }
    }
    out
}

/// Birleştirme reddedilir: satırlar aynı güne ve projeye ait değil ya da ikiden az.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    #[error("birleştirmek için en az iki satır seç")]
    TooFew,
    #[error("yalnızca aynı günün satırları birleştirilir")]
    Days,
    #[error("yalnızca aynı projenin satırları birleştirilir")]
    Projects,
}

/// Satırları tek satırda birleştirir (aynı gün ve proje): en erken başlangıç, toplam süre ve
/// aralıklar; tür, taraf ve birim en çok saati olanınki (eşitlikte erkenin), açıklamalar
/// sırayla ve tekrarsız. Saat, satırların saati süreden yuvarlandıysa toplam sürenin
/// yuvarlanmışıdır (küçük parçaların ayrı ayrı yukarı yuvarlanması birikmesin); biri elle
/// değiştirildiyse saatlerin toplamı. Aralıkları bilinmeyen eski satır varsa sonuç da öyledir
/// (işi saatiyle düşülmeye devam eder).
pub fn merge(rows: &[TimesheetEntry]) -> Result<TimesheetEntry, MergeError> {
    let mut rows: Vec<&TimesheetEntry> = rows.iter().collect();
    rows.sort_by_key(|e| e.start);
    let (Some(first), true) = (rows.first().copied(), rows.len() >= 2) else {
        return Err(MergeError::TooFew);
    };
    if rows.iter().any(|e| e.date != first.date) {
        return Err(MergeError::Days);
    }
    if rows.iter().any(|e| e.project_id != first.project_id) {
        return Err(MergeError::Projects);
    }
    let rounded = rows
        .iter()
        .all(|e| (e.hours - round_quarter(e.worked())).abs() < 1e-9);
    let worked: f64 = rows.iter().map(|e| e.worked()).sum();
    let hours = if rounded {
        round_quarter(worked)
    } else {
        rows.iter().map(|e| e.hours).sum()
    };
    let mut details: Vec<&str> = Vec::new();
    for e in &rows {
        let d = e.details.trim();
        if !d.is_empty() && !details.iter().any(|x| x.to_lowercase() == d.to_lowercase()) {
            details.push(d);
        }
    }
    let coverage = rows
        .iter()
        .map(|e| e.coverage.as_deref().map(from_coverage))
        .collect::<Option<Vec<_>>>()
        .map(|all| to_coverage(&coalesce(all.concat())));
    Ok(TimesheetEntry {
        date: first.date,
        start: first.start,
        hours,
        actual_hours: rows
            .iter()
            .any(|e| e.actual_hours.is_some())
            .then_some(worked),
        kind: dominant(&rows, |e| e.kind),
        details: details.join("; "),
        party: dominant(&rows, |e| e.party.clone()),
        project_id: first.project_id.clone(),
        division: dominant(&rows, |e| e.division.clone()),
        coverage,
    })
}

/// Satırların saatine göre en ağır değer; eşitlikte ilk görülen (satırlar başlangıca göre sıralı).
fn dominant<K: PartialEq>(rows: &[&TimesheetEntry], key: impl Fn(&TimesheetEntry) -> K) -> K {
    let mut totals: Vec<(K, f64)> = Vec::new();
    for e in rows {
        let k = key(e);
        match totals.iter_mut().find(|(x, _)| *x == k) {
            Some((_, h)) => *h += e.hours,
            None => totals.push((k, e.hours)),
        }
    }
    let mut best = 0;
    for (i, (_, h)) in totals.iter().enumerate().skip(1) {
        if *h > totals[best].1 + 1e-9 {
            best = i;
        }
    }
    totals.swap_remove(best).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Rule, RuleField, Tag, TagKind};
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

    fn mapping(project: &str, division: &str) -> ProjectMapping {
        ProjectMapping {
            project_id: project.into(),
            division: division.into(),
            party: None,
            default_details: None,
        }
    }

    fn setup() -> (
        Classifier,
        HashMap<String, String>,
        TimesheetConfig,
        Timesheet,
    ) {
        let tag = |id: &str, name: &str| Tag {
            id: id.into(),
            kind: TagKind::Project,
            name: name.into(),
            color: 1,
        };
        let classifier = Classifier::new(
            &[
                tag("tru", "Trumore"),
                tag("sync", "Int.Work.Sync."),
                tag("kum", "Kum"),
            ],
            &[
                Rule {
                    id: "r".into(),
                    tag_id: "tru".into(),
                    field: RuleField::Title,
                    pattern: "trumore".into(),
                },
                Rule {
                    id: "k".into(),
                    tag_id: "kum".into(),
                    field: RuleField::Title,
                    pattern: "kum".into(),
                },
            ],
        );
        let names = HashMap::from([
            ("tru".to_string(), "Trumore".to_string()),
            ("sync".to_string(), "Int.Work.Sync.".to_string()),
            ("kum".to_string(), "Kum".to_string()),
        ]);
        let sheet = Timesheet {
            id: "togg".into(),
            company: "Togg".into(),
            default_party: "ADBA".into(),
            // Kum projesi bu çizelgeye bağlı değil: önerilmez.
            projects: vec![mapping("tru", ""), mapping("sync", "Int.Work.Sync.")],
            ..Default::default()
        };
        (classifier, names, TimesheetConfig::default(), sheet)
    }

    /// Tek günün (kaydedilmiş satırı olmayan) önerileri.
    fn day(
        sessions: &[Session],
        meetings: &[(Meeting, String)],
        classifier: &Classifier,
        names: &HashMap<String, String>,
        config: &TimesheetConfig,
        sheet: &Timesheet,
    ) -> Vec<TimesheetEntry> {
        let p = pieces(sessions, meetings, classifier, config, t(-540), t(900));
        propose(&p, names, sheet, &HashMap::new())
    }

    #[test]
    fn groups_by_project_and_kind_with_short_gaps() {
        let (classifier, names, config, sheet) = setup();
        let sessions = [
            s("Figma", "Trumore Loyalty UI/UX — Figma", 0, 50, None),
            s("Slack", "#genel", 50, 55, None), // projesiz: kayda girmez
            s("Figma", "Trumore Loyalty UI/UX — Figma", 55, 90, None), // 5 dk boşluk: aynı kayıt
            s("us.zoom.xos", "Zoom Meeting", 90, 120, Some("tru")), // toplantı: ayrı kayıt
            s("Figma", "Trumore Pitchdeck — Figma", 150, 170, None), // 60 dk boşluk: yeni kayıt
            s("Slack", "sync", 170, 173, Some("sync")), // 3 dk: çok kısa
            s("Code", "kum — main.rs", 180, 240, None), // başka çizelgenin projesi
        ];
        let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
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
        // Kaydın aralıkları boşluklar olmadan saklanır.
        assert_eq!(
            got[0].spans(),
            vec![(t(0), t(50)), (t(55), t(90))],
            "aradaki projesiz 5 dk kayda girmez"
        );
    }

    #[test]
    fn only_the_sheets_projects_are_proposed() {
        let (classifier, names, config, mut sheet) = setup();
        let sessions = [
            s("Figma", "Trumore — Figma", 0, 60, None),
            s("Code", "kum — main.rs", 60, 120, None),
        ];
        let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].project_id, "tru");
        // Kişisel projenin çizelgesi: yalnızca Kum.
        sheet.projects = vec![mapping("kum", "")];
        let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].project_id.as_str(), got[0].division.as_str()),
            ("kum", "Kum")
        );
        // Projesi olmayan çizelge hiçbir şey almaz.
        sheet.projects.clear();
        assert!(day(&sessions, &[], &classifier, &names, &config, &sheet).is_empty());
    }

    #[test]
    fn saved_spans_are_not_proposed_again() {
        let (classifier, names, config, sheet) = setup();
        let sessions = [
            s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
            s("Figma", "Trumore Rapor — Figma", 120, 180, None),
        ];
        let p = pieces(&sessions, &[], &classifier, &config, t(-540), t(900));
        let all = propose(&p, &names, &sheet, &HashMap::new());
        assert_eq!(all.len(), 2);
        // İkinci satır kaydedildi (düzenlendi): ilki önerilmeye devam eder, ikincisi gelmez.
        let saved = HashMap::from([("tru".to_string(), all[1].spans())]);
        let left = propose(&p, &names, &sheet, &saved);
        assert_eq!(left, vec![all[0].clone()]);

        // Sabah unutulan iş sonradan projeye atanınca kendi saatinde gelir.
        let mut later = sessions.to_vec();
        later.push(s("Mail", "Rapor taslağı", -120, -60, Some("tru")));
        let p = pieces(&later, &[], &classifier, &config, t(-540), t(900));
        let saved = HashMap::from([(
            "tru".to_string(),
            coalesce([all[0].spans(), all[1].spans()].concat()),
        )]);
        let got = propose(&p, &names, &sheet, &saved);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].start.format("%H:%M").to_string(), "07:00");
        assert_eq!(got[0].details, "Rapor taslağı");

        // Satır kaydedilirken süren işin kısa kuyruğu önerilmez; uzayınca önerilir.
        let mut tail = sessions.to_vec();
        tail.push(s("Figma", "Trumore Rapor — Figma", 180, 190, None));
        let p = pieces(&tail, &[], &classifier, &config, t(-540), t(900));
        let saved = HashMap::from([(
            "tru".to_string(),
            coalesce([all[0].spans(), all[1].spans()].concat()),
        )]);
        assert!(propose(&p, &names, &sheet, &saved).is_empty());
        tail.push(s("Figma", "Trumore Rapor — Figma", 190, 200, None));
        let p = pieces(&tail, &[], &classifier, &config, t(-540), t(900));
        let got = propose(&p, &names, &sheet, &saved);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].start.format("%H:%M").to_string(), "12:00");
        assert_eq!(got[0].spans(), vec![(t(180), t(200))]);
    }

    #[test]
    fn a_short_meeting_after_a_saved_one_is_still_proposed() {
        let (classifier, names, config, sheet) = setup();
        let first = (
            meeting("a", "Trumore haftalık", 0, 60, true),
            "tru".to_string(),
        );
        let second = (
            meeting("b", "Trumore kısa", 60, 70, true),
            "tru".to_string(),
        );
        let p = pieces(&[], &[first, second], &classifier, &config, t(-540), t(900));
        let all = propose(&p, &names, &sheet, &HashMap::new());
        assert_eq!(all.len(), 2);
        let saved = HashMap::from([("tru".to_string(), all[0].spans())]);
        assert_eq!(propose(&p, &names, &sheet, &saved), vec![all[1].clone()]);
        // Kaydedilmiş toplantı takvimde 10 dakika uzadı: uç yeni iş sayılmaz.
        let longer = (
            meeting("a", "Trumore haftalık", 0, 70, true),
            "tru".to_string(),
        );
        let p = pieces(&[], &[longer], &classifier, &config, t(-540), t(900));
        assert!(propose(&p, &names, &sheet, &saved).is_empty());
    }

    #[test]
    fn stale_rows_are_found_and_refreshed() {
        let (classifier, names, config, sheet) = setup();
        let sessions = vec![
            s("Mail", "Rapor", 0, 30, Some("tru")),
            s("Mail", "Rapor", 30, 60, Some("tru")),
        ];
        let p = pieces(&sessions, &[], &classifier, &config, t(-540), t(900));
        let mut row = propose(&p, &names, &sheet, &HashMap::new()).remove(0);
        row.details = "Elle yazıldı".into();
        assert_eq!(stale_hours(&p, &row), None);
        // İlk yarım saat raporda başka projeye alındı.
        let mut moved = sessions.clone();
        moved[0].project_id = Some("kum".into());
        let p = pieces(&moved, &[], &classifier, &config, t(-540), t(900));
        assert_eq!(stale_hours(&p, &row), Some(0.5));
        let fresh = refreshed(&p, &row).unwrap();
        assert_eq!(fresh.start.format("%H:%M").to_string(), "09:30");
        assert_eq!((fresh.hours, fresh.actual_hours), (0.5, Some(0.5)));
        assert_eq!(fresh.details, "Elle yazıldı");
        assert_eq!(fresh.spans(), vec![(t(30), t(60))]);
        // Elle değiştirilen başlangıç korunur.
        let typed = TimesheetEntry {
            start: NaiveTime::from_hms_opt(8, 45, 0).unwrap(),
            ..row.clone()
        };
        assert_eq!(refreshed(&p, &typed).unwrap().start, typed.start);
        // Hepsi gitti: satır kalmaz.
        moved[1].project_id = Some("kum".into());
        let p = pieces(&moved, &[], &classifier, &config, t(-540), t(900));
        assert_eq!(stale_hours(&p, &row), Some(0.0));
        assert_eq!(refreshed(&p, &row), None);
        // Elle eklenen satır takipten bağımsızdır.
        let manual = TimesheetEntry {
            coverage: Some(Vec::new()),
            ..row
        };
        assert_eq!(stale_hours(&p, &manual), None);
    }

    #[test]
    fn rows_merge_into_one() {
        let (classifier, names, config, sheet) = setup();
        let sessions = [
            s("Figma", "Trumore Loyalty — Figma", 0, 10, None),
            s("Figma", "Trumore Rapor — Figma", 60, 70, None),
            s("Figma", "Trumore Loyalty — Figma", 120, 160, None),
            s("us.zoom.xos", "Zoom", 200, 205, Some("tru")),
        ];
        let rows = day(&sessions, &[], &classifier, &names, &config, &sheet);
        assert_eq!(rows.len(), 4);
        // 10 + 10 + 40 + 5 dk: ayrı ayrı yuvarlanınca 0,25 × 3 + 0,75 = 1,50; birleşince 1,00.
        let merged = merge(&rows).unwrap();
        assert_eq!(merged.start.format("%H:%M").to_string(), "09:00");
        assert_eq!(merged.hours, 1.0);
        assert!((merged.worked() - 65.0 / 60.0).abs() < 1e-9);
        assert_eq!(merged.kind, EntryKind::Working);
        assert_eq!(
            merged.details, "Trumore Loyalty; Trumore Rapor",
            "tekrar eden açıklama bir kez"
        );
        assert_eq!(
            merged.spans(),
            vec![
                (t(0), t(10)),
                (t(60), t(70)),
                (t(120), t(160)),
                (t(200), t(205))
            ]
        );
        // Elle değiştirilen saat korunur: toplam saat.
        let mut edited = rows[..2].to_vec();
        edited[1].hours = 1.0;
        assert_eq!(merge(&edited).unwrap().hours, 1.25);
        // Tür, en çok saati olan.
        let mut kinds = rows[..2].to_vec();
        kinds[1].kind = EntryKind::F2F;
        kinds[1].hours = 2.0;
        assert_eq!(merge(&kinds).unwrap().kind, EntryKind::F2F);
        // Eski (aralığı bilinmeyen) satır varsa sonuç da öyle.
        let mut legacy = rows[..2].to_vec();
        legacy[0].coverage = None;
        assert_eq!(merge(&legacy).unwrap().coverage, None);

        assert_eq!(merge(&rows[..1]), Err(MergeError::TooFew));
        let mut other = rows[..2].to_vec();
        other[1].project_id = "sync".into();
        assert_eq!(merge(&other), Err(MergeError::Projects));
        other[1].date = other[1].date.succ_opt().unwrap();
        assert_eq!(merge(&other), Err(MergeError::Days));
    }

    #[test]
    fn file_rows_link_to_exported_rows() {
        let (classifier, names, config, sheet) = setup();
        let sessions = [
            s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
            s("Figma", "Trumore Rapor — Figma", 120, 180, None),
            s("Figma", "Trumore Sunum — Figma", 240, 270, None),
        ];
        let rows = day(&sessions, &[], &classifier, &names, &config, &sheet);
        let entries: Vec<&TimesheetEntry> = rows.iter().collect();
        let written: Vec<FileRow> = rows
            .iter()
            .enumerate()
            .map(|(i, e)| FileRow::of(e, i as u32 + 2))
            .collect();
        // Elle girilmiş satır (Kum'da yok) ve dosyada değiştirilmiş açıklama.
        let mut file = written.clone();
        file[1].details = "Aylık rapor (düzeltildi)".into();
        file[1].hours = Some(1.5);
        let manual = FileRow {
            row: 9,
            date: rows[0].date,
            start: NaiveTime::from_hms_opt(16, 0, 0),
            hours: Some(1.0),
            kind: "F2F".into(),
            details: "Atölye".into(),
            ..Default::default()
        };
        file.push(manual.clone());
        // Sunumun başlangıcı dosyada değişti: eşlenmez.
        file[2].start = NaiveTime::from_hms_opt(8, 0, 0);
        file[2].kind = "Online".into();
        file[2].details = "başka".into();
        assert_eq!(
            link_file_rows(&file, &entries),
            [Some(0), Some(1), None, None]
        );
        assert!(written[0].matches(&rows[0]) && !file[1].matches(&rows[1]));
        // Değiştirilen satır Kum'a dosyadaki haliyle geçer; aralıkları korunur.
        let synced = file[1].apply(&rows[1]).unwrap();
        assert_eq!(
            (synced.details.as_str(), synced.hours),
            ("Aylık rapor (düzeltildi)", 1.5)
        );
        assert_eq!(synced.coverage, rows[1].coverage);
        assert!(file[1].matches(&synced));
        // Türü tanınmayan ya da saati boş satır Kum'a geçmez.
        assert!(
            FileRow {
                hours: None,
                ..file[1].clone()
            }
            .apply(&rows[1])
            .is_none()
        );
        // Aynı satır iki kez eşlenmez.
        let twice = vec![written[0].clone(), written[0].clone()];
        assert_eq!(link_file_rows(&twice, &entries), [Some(0), None]);
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
        let (classifier, names, config, mut sheet) = setup();
        sheet.projects[1].party = Some("Togg".into());
        let mut meeting = s("kum.manual/Workshop", "Workshop", 0, 60, Some("sync"));
        meeting.app_name = "Workshop".into();
        let got = day(&[meeting], &[], &classifier, &names, &config, &sheet);
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
        let (classifier, names, config, mut sheet) = setup();
        sheet.projects[0].default_details = Some("  Trumore danışmanlık ".into());
        let sessions = [
            // Başlıktan açıklama çıkar: hazır metin kullanılmaz.
            s("Figma", "Trumore Pitchdeck — Figma", 0, 30, None),
            // Toplantı uygulaması: açıklama boş kalır, hazır metin girer.
            s("us.zoom.xos", "Zoom Meeting", 30, 60, Some("tru")),
            // Hazır metni olmayan projenin boş açıklaması boş kalır.
            s("us.zoom.xos", "Zoom Meeting", 60, 90, Some("sync")),
        ];
        let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
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
        // Aralıkları olmayan (önceki sürümün) kayıt da okunur.
        let e: TimesheetEntry = serde_json::from_str(
            r#"{"date":"2026-10-01","start":"09:00:00","hours":1,"kind":"Working",
                "details":"","party":"","projectId":"a","division":"A"}"#,
        )
        .unwrap();
        assert_eq!(e.coverage, None);
    }

    #[test]
    fn config_keeps_each_project_in_one_sheet() {
        let mut config = TimesheetConfig {
            timesheets: vec![
                Timesheet {
                    id: "a".into(),
                    projects: vec![mapping("p1", ""), mapping("p2", "")],
                    divisions: vec![" Trumore ".into(), "trumore".into(), "".into()],
                    ..Default::default()
                },
                Timesheet {
                    id: "a".into(),
                    projects: vec![mapping("p2", ""), mapping("p3", "")],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        config.normalize();
        let [a, b] = &config.timesheets[..] else {
            panic!("iki çizelge");
        };
        assert_ne!(a.id, b.id);
        assert_eq!(a.divisions, ["Trumore"]);
        assert!(a.includes("p2") && !b.includes("p2") && b.includes("p3"));
        assert_eq!(
            config.timesheet_of("p3").map(|t| t.id.as_str()),
            Some(b.id.as_str())
        );
        assert!(config.timesheet_of("p4").is_none());
    }

    #[test]
    fn assigned_idle_time_is_face_to_face() {
        let (c, names, config, sheet) = setup();
        let mut away = Session::idle(t(0), t(60));
        away.project_id = Some("tru".into());
        let out = day(&[away], &[], &c, &names, &config, &sheet);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, EntryKind::F2F);
        assert_eq!(out[0].hours, 1.0);
    }

    #[test]
    fn browser_meetings_are_online() {
        let (classifier, names, config, sheet) = setup();
        let got = day(
            &[s("com.google.Chrome", "Meet - Trumore weekly", 0, 30, None)],
            &[],
            &classifier,
            &names,
            &config,
            &sheet,
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
        let (classifier, names, config, sheet) = setup();
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
        let got = day(&sessions, &meetings, &classifier, &names, &config, &sheet);
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
        // Toplantı başka projeye atanınca kaydedilmiş satırı takipte değişmiş olur.
        let p = pieces(&sessions, &meetings, &classifier, &config, t(-540), t(900));
        let saved_meeting = &got[1];
        assert_eq!(stale_hours(&p, saved_meeting), None);
        let moved = [
            (meetings[0].0.clone(), "kum".to_string()),
            meetings[1].clone(),
        ];
        let p = pieces(&sessions, &moved, &classifier, &config, t(-540), t(900));
        assert_eq!(stale_hours(&p, saved_meeting), Some(0.0));
    }

    #[test]
    fn meeting_project_prefers_assignment_then_rules() {
        let (classifier, _, _, _) = setup();
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
            coverage: None,
        }
    }

    #[test]
    fn hours_are_rounded_to_quarters_and_actual_is_kept() {
        let (classifier, names, config, sheet) = setup();
        let sessions = [
            s("Figma", "Trumore — Figma", 0, 67, None), // 1 sa 7 dk → 1,00
            s("Figma", "Trumore — Figma", 120, 128, None), // 8 dk → 0,25 (en az)
            s("Figma", "Trumore — Figma", 200, 253, None), // 53 dk → 1,00 (0,88 → 1)
        ];
        let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
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
    fn legacy_rows_are_not_proposed_again() {
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
        // Hiç eski satır yoksa öneriler aynen.
        assert_eq!(without_legacy(&proposed, &[]).len(), 4);
        // Aktarırken 09:10 kaydının saati 09:00'a çekilmiş ve 0,75'e yuvarlanmış; 13:00 kaydı
        // aktarıldığında 1 saatti, sonra 2 saate uzadı. Düzenlenmiş kayıt tekrar önerilmez;
        // toplam korunur (aktarılan 1,75 + önerilen 1,08 = takip edilen 2,83). Toplantı ve
        // b projesi aktarılmadı: aynen kalır.
        let exported = [
            entry("a", Working, 9, 0, 0.75),
            entry("a", Working, 13, 0, 1.0),
        ];
        assert_eq!(
            short(without_legacy(&proposed, &exported)),
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
            without_legacy(&proposed, &exported)
                .iter()
                .all(|e| e.project_id != "a" || e.kind != Working)
        );
        // Tamamı aktarılmış gün: yeni bir şey yok.
        assert!(without_legacy(&proposed, &proposed).is_empty());

        // Kısmen düşülen önerinin aralıkları da düşülen süre kadar kısalır.
        let mut long = entry("a", Working, 9, 0, 2.0);
        long.actual_hours = Some(2.0);
        long.coverage = Some(to_coverage(&[(t(0), t(60)), (t(90), t(150))]));
        let rest = without_legacy(&[long], &[entry("a", Working, 9, 0, 1.25)]);
        assert_eq!(rest[0].spans(), vec![(t(105), t(150))]);
        assert_eq!(rest[0].start.format("%H:%M").to_string(), "10:15");
    }

    #[test]
    fn spans_coalesce() {
        assert_eq!(
            coalesce(vec![
                (t(30), t(40)),
                (t(0), t(10)),
                (t(10), t(20)),
                (t(35), t(50)),
                (t(60), t(60)),
            ]),
            vec![(t(0), t(20)), (t(30), t(50))]
        );
        let spans = vec![(t(0), t(20)), (t(30), t(50))];
        assert_eq!(from_coverage(&to_coverage(&spans)), spans);
        assert_eq!(total(&spans), Duration::minutes(40));
    }
}
