//! Zaman çizelgesi: günün projeye atanmış süresini firmaya gönderilecek iş kayıtlarına
//! (tarih, başlangıç, saat, tür, açıklama, taraf, birim) dönüştürür.
//!
//! Her zaman çizelgesi ([`Timesheet`]) bir firmanın dosyasıdır (Excel ya da Google Sheets) ve
//! yalnızca kendisine bağlanan projelerin işini alır: Togg'un tablosuna yalnızca Togg'a bağlı
//! projelerin süresi yazılır. Bir proje en çok bir zaman çizelgesine bağlanır.
//!
//! Takvimdeki blok zaman çizelgesindeki satırdır: raporda (gün, hafta, ay) yapılan atama ve
//! düzenlemeler doğrudan satırlara yansır. Yalnızca bir projeye düşen süre iş sayılır; projesi
//! olan bloğun içindeki atanmamış süre de o projenindir. Aynı projedeki ardışık oturumlar
//! (bilgisayarda, uzakta ya da görüşmede), aradaki boşluk `MERGE_GAP`'i geçmedikçe tek kayıt
//! olur; türü en çok süreninkidir. Kaydın gerçek süresi boşluklar değil, oturumların toplam
//! süresidir. Firmaya giden saat bu sürenin çeyrek saate
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
    /// Davet metninden gündem (Teams bloğu, bağlantılar atılmış); yoksa boş.
    pub agenda: String,
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
    /// Takvim bloğu burada elle bölündü ([`crate::blocks::Activity::block_start`]): projenin
    /// açık kaydı kapanır, yeni satır başlar.
    pub block_start: bool,
}

/// Günün zaman çizelgesine giren süresi: projesi belli toplantılar ([`meeting_project`]) ve
/// oturumlar, `day_start`–`day_end` (yerel gün) sınırına kırpılmış. Üst üste binen
/// toplantılarda ortak süre ilkine yazılır; toplantı süresince takip edilen iş toplantının
/// kaydında sayılır (aynı saat iki kez yazılmasın).
///
/// Takvimdeki blok zaman çizelgesindeki satırdır: projesi olan bloğun ([`crate::blocks`])
/// içindeki atanmamış süre de o projenin işidir (takvimde bloğun süresine girer). Başka projeye
/// ya da projesize atanmış süre bloğun projesine geçmez.
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
            block_start: false,
        }));
    }
    let classes: Vec<_> = sessions.iter().map(|s| classifier.classify(s)).collect();
    let blocks = day_blocks(sessions, &classes, day_start, day_end);
    for (s, class) in sessions.iter().zip(&classes) {
        let (a, b) = (s.started_at.max(day_start), s.ended_at.min(day_end));
        if b <= a {
            continue;
        }
        // Projesi olan süre kendi projesinin; atanmamış süre içinde geçtiği bloğun projesinin.
        let owned: Vec<(DateTime<Utc>, DateTime<Utc>, String)> = match &class.project {
            Some(p) => vec![(a, b, p.clone())],
            None if s.project_id.as_deref() == Some(crate::classify::NO_PROJECT) => continue,
            None => blocks
                .iter()
                .filter_map(|(c, d, p)| {
                    let (x, y) = (a.max(*c), b.min(*d));
                    (y > x).then(|| (x, y, p.clone()))
                })
                .collect(),
        };
        let kind = kind_of(s, config);
        let title = clean_title(&s.title, &s.app_name);
        for (a, b, project) in owned {
            let parts = covered.iter().fold(vec![(a, b)], |p, &c| subtract(p, c));
            out.extend(parts.into_iter().map(|(start, end)| Piece {
                project: project.clone(),
                kind,
                start,
                end,
                title: title.clone(),
                meeting: None,
                block_start: crate::blocks::Activity::starts_block(s, start),
            }));
        }
    }
    out
}

/// Günün takvim blokları (raporla aynı hesap) ve projeleri; projesi olmayan bloklar atlanır.
fn day_blocks(
    sessions: &[Session],
    classes: &[crate::classify::Classification],
    day_start: DateTime<Utc>,
    day_end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>, String)> {
    let mut items: Vec<crate::blocks::Activity> = sessions
        .iter()
        .zip(classes)
        .filter_map(|(s, class)| {
            let (a, b) = (s.started_at.max(day_start), s.ended_at.min(day_end));
            (b > a).then(|| crate::blocks::Activity {
                start: a,
                end: b,
                app_id: &s.app_id,
                app_name: &s.app_name,
                category: class.category.as_deref(),
                project: class.project.as_deref(),
                block_start: crate::blocks::Activity::starts_block(s, a),
            })
        })
        .collect();
    items.sort_by_key(|i| i.start);
    crate::blocks::analyze(&items)
        .blocks
        .into_iter()
        .filter_map(|b| Some((b.start, b.end, b.project_id?)))
        .collect()
}

/// Parçalardan `sheet`'e bağlı projelerin iş kaydı önerileri; başlangıca göre sıralı.
/// `saved` proje başına kaydedilmiş (silinmişler dahil) satırların aralıklarıdır, satır satır:
/// bu süre yeniden önerilmez. Kaydedilmiş satırın artığı `MIN_REMAINDER`'dan kısaysa önerilmez:
/// satır kaydedilirken süren işin birkaç dakikası (satıra `MERGE_GAP`'ten yakın oturumlar) ya da
/// bir kısmı kaydedilmiş toplantının uzayan ucu yeni iş sayılmaz. Satırın ilk ve son aralığı
/// arasındaki boşluklara düşen süre ise satır kaydedilirken projenin değildi (raporda sonradan
/// atandı): en az `MIN_ENTRY` ise kısa da olsa önerilir. Kaydedilmiş toplantının ardından gelen
/// ayrı bir toplantı da kısa da olsa önerilir.
pub fn propose(
    pieces: &[Piece],
    project_names: &HashMap<String, String>,
    sheet: &Timesheet,
    saved: &HashMap<String, Vec<Vec<Interval>>>,
) -> Vec<TimesheetEntry> {
    struct Run {
        project: String,
        kind: EntryKind,
        /// Türlere göre süre: oturum kaydının türü en çok sürenidir.
        kinds: HashMap<EntryKind, Duration>,
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
                kinds: HashMap::new(),
                meeting: p.meeting.is_some(),
                start: at,
                end: at,
                worked: Duration::zero(),
                titles: HashMap::new(),
                spans: Vec::new(),
                trimmed: false,
            }
        }

        fn add(&mut self, a: DateTime<Utc>, b: DateTime<Utc>, p: &Piece, trimmed: bool) {
            *self.kinds.entry(p.kind).or_insert(Duration::zero()) += b - a;
            self.trimmed |= trimmed;
            self.end = self.end.max(b);
            self.worked += b - a;
            // Toplantı uygulamasının başlığı ("Zoom Meeting") açıklama değildir; takvimden gelen
            // toplantının konusu ise açıklamadır.
            if p.kind != EntryKind::Online || p.meeting.is_some() {
                *self
                    .titles
                    .entry(p.title.clone())
                    .or_insert(Duration::zero()) += b - a;
            }
            self.spans.push((a, b));
        }

        /// Oturum kaydının türü en çok sürenidir; eşitlikte bilgisayarda çalışma.
        fn settle_kind(&mut self) {
            self.kind = [EntryKind::Working, EntryKind::Online, EntryKind::F2F]
                .into_iter()
                .max_by_key(|k| {
                    (
                        self.kinds.get(k).copied().unwrap_or_default(),
                        *k == EntryKind::Working,
                    )
                })
                .unwrap_or(self.kind);
        }
    }

    // Proje başına kaydedilmiş süre ve satırların kapladığı aralıklar (ilk aralığın başından
    // sonuncunun sonuna).
    let cuts: HashMap<&str, (Vec<Interval>, Vec<Interval>)> = saved
        .iter()
        .map(|(project, rows)| {
            let hulls = rows
                .iter()
                .filter_map(|spans| {
                    Some((
                        spans.iter().map(|s| s.0).min()?,
                        spans.iter().map(|s| s.1).max()?,
                    ))
                })
                .collect();
            (project.as_str(), (coalesce(rows.concat()), hulls))
        })
        .collect();
    let none: Vec<Interval> = Vec::new();
    let mut parts: Vec<(DateTime<Utc>, DateTime<Utc>, &Piece, bool)> = pieces
        .iter()
        .filter(|p| sheet.includes(&p.project))
        .flat_map(|p| {
            let left = cuts
                .get(p.project.as_str())
                .map_or(&none, |c| &c.0)
                .iter()
                .fold(vec![(p.start, p.end)], |v, &c| subtract(v, c));
            let trimmed = total(&left) < p.end - p.start;
            left.into_iter().map(move |(a, b)| (a, b, p, trimmed))
        })
        .collect();
    parts.sort_by_key(|(a, _, _, _)| *a);

    // Her toplantı kendi kaydı; oturumlarda her proje için açık kayıt, araya başka iş girse de
    // boşluk kısaysa sürer. Takvimdeki blok gibi bilgisayarda, uzakta ve görüşmede geçen süre
    // aynı kayıttadır.
    let mut meetings: Vec<(usize, Run)> = Vec::new();
    let mut open: HashMap<String, Run> = HashMap::new();
    let mut runs: Vec<Run> = Vec::new();
    for (a, b, p, trimmed) in parts {
        if let Some(i) = p.meeting {
            match meetings.iter_mut().find(|(j, _)| *j == i) {
                Some((_, run)) => run.add(a, b, p, trimmed),
                None => {
                    let mut run = Run::new(p, a);
                    run.add(a, b, p, trimmed);
                    meetings.push((i, run));
                }
            }
            continue;
        }
        let key = p.project.clone();
        if let Some(run) = open.get(&key)
            && (a - run.end > MERGE_GAP || (p.block_start && a == p.start))
        {
            let mut run = open.remove(&key).expect("az önce bulundu");
            run.settle_kind();
            runs.push(run);
        }
        open.entry(key)
            .or_insert_with(|| Run::new(p, a))
            .add(a, b, p, trimmed);
    }
    for run in open.values_mut() {
        run.settle_kind();
    }
    runs.extend(open.into_values());
    runs.extend(meetings.into_iter().map(|(_, run)| run));

    // Kaydedilmiş satırın artığı mı: oturumlarda satıra yakın iş, toplantıda aynı toplantının ucu.
    // Satırın kapladığı aralığa sonradan atanmış en az `MIN_ENTRY` iş varsa artık değildir.
    let remainder = |r: &Run| {
        if r.meeting {
            return r.trimmed;
        }
        cuts.get(r.project.as_str()).is_some_and(|(cut, hulls)| {
            let near = r.spans.iter().any(|&(a, b)| {
                cut.iter()
                    .any(|&(c, d)| a <= d + MERGE_GAP && c <= b + MERGE_GAP)
            });
            let inside: Duration = r
                .spans
                .iter()
                .flat_map(|&(a, b)| hulls.iter().map(move |&(c, d)| (a.max(c), b.min(d))))
                .filter(|(a, b)| b > a)
                .map(|(a, b)| b - a)
                .sum();
            near && inside < MIN_ENTRY
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
            let details = describe(r.titles);
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
/// önerilerden düşer: aynı gün ve saatte (dakikasıyla) başlayan öneri varsa eski satırın gerçek
/// süresi (yoksa yazılan saat) önce ondan düşülür (türü elle değiştirilmiş toplantı satırı yine
/// kendi toplantısını kapsar); kalanı her (proje, tür) için o türün en erken önerilerinden
/// düşülür. Yalnızca artan süre kalır (eski + önerilen = takip edilen); kalanın saati yeniden
/// yuvarlanır. Satırın saati ya da süresi elle değiştirilmiş olsa da iş ikinci kez önerilmez.
/// Kısmen düşülen önerinin kalanı `MIN_REMAINDER`'dan kısaysa atılır, değilse başlangıcı ve
/// aralıkları düşülen süre kadar ileri alınır.
pub fn without_legacy(
    proposed: &[TimesheetEntry],
    legacy: &[TimesheetEntry],
) -> Vec<TimesheetEntry> {
    let min_rest = MIN_REMAINDER.num_seconds() as f64 / 3600.0;
    let mut sorted: Vec<&TimesheetEntry> = proposed.iter().collect();
    sorted.sort_by_key(|e| e.start);
    let minute = |t: NaiveTime| (t.hour(), t.minute());
    // Önerilerden düşülen süre; aynı saatte başlayan eski satırlar önce.
    let mut used: Vec<f64> = vec![0.0; sorted.len()];
    let mut budget: HashMap<(&str, EntryKind), f64> = HashMap::new();
    for x in legacy {
        let mut left = x.worked();
        if let Some(i) = sorted.iter().enumerate().position(|(i, e)| {
            e.project_id == x.project_id
                && e.date == x.date
                && minute(e.start) == minute(x.start)
                && used[i] < e.worked()
        }) {
            let take = left.min(sorted[i].worked() - used[i]);
            used[i] += take;
            left -= take;
        }
        *budget.entry((x.project_id.as_str(), x.kind)).or_default() += left;
    }
    let mut out = Vec::new();
    for (e, before) in sorted.into_iter().zip(used) {
        let left = budget.entry((e.project_id.as_str(), e.kind)).or_default();
        let more = left.min(e.worked() - before);
        *left -= more;
        let used = before + more;
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
/// Projede `MIN_ENTRY`'den az süre kaldıysa `None` (birkaç saniye çeyrek saate yuvarlanıp
/// satır olarak kalmasın; öneriler de bu kadar kısa satır çıkarmaz).
pub fn refreshed(pieces: &[Piece], entry: &TimesheetEntry) -> Option<TimesheetEntry> {
    let spans = entry.spans();
    let left = still_covered(pieces, &entry.project_id, &spans);
    if total(&left) < MIN_ENTRY {
        return None;
    }
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

    /// Kum'un satırı dosyada değiştirilmiş hali olabilir: aynı gün ve başlangıç; açıklama aynı
    /// (birimi dosyada değiştirilmiş olabilir) ya da tür ve birim aynı. Yalnızca türü tutan,
    /// birimi başka bir satır aynı saatte elle girilmiş başka bir iştir; eşlenseydi değerleri
    /// Kum'daki kaydın üzerine yazılırdı ([`FileRow::apply`]).
    fn near(&self, e: &TimesheetEntry) -> bool {
        self.same_start(e)
            && (self.details.trim() == e.details.trim()
                || (self.kind.trim() == e.kind.label()
                    && self.division.trim().eq_ignore_ascii_case(e.division.trim())))
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
mod tests;
