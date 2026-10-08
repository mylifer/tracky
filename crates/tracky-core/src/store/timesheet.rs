//! Zaman çizelgesi: ayarlar (firmaların çizelgeleri), toplantı atamaları, günün satırları
//! (kaydedilmiş satırlar ve takipten gelen canlı öneriler) ve satırların düzenlenmesi.
//!
//! Canlı öneri ilk kez düzenlenince, birleştirilince, gizlenince ya da aktarılınca kaydedilir;
//! kaydedilen satır kapsadığı takip aralıklarını saklar ve bu aralıklar yeniden önerilmez
//! ([`crate::timesheet::propose`]).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::params;
use serde::Deserialize;
use uuid::Uuid;

use super::{Result, Store, StoreError, from_ms, ms};
use crate::classify::{Classifier, TagKind};
use crate::meeting_suggest::{MeetingSuggester, ProjectInfo, SuggestInput};
use crate::timesheet::{
    self, EntryKind, Interval, Meeting, MeetingProject, Piece, ProjectMapping, Timesheet,
    TimesheetConfig, TimesheetEntry,
};

/// Toplantı önerilerinde projelerin kullanımına bakılan dönem (gün; zaman çizelgesi kayıtları).
const SUGGEST_USAGE_DAYS: u64 = 90;

/// Zaman çizelgesi ayarları.
const TIMESHEET_KEY: &str = "timesheet";
/// Takvim toplantı serilerinin elle verilen projesi (UID → proje; `null`: yoksay).
const MEETING_ASSIGNMENTS_KEY: &str = "meeting_assignments";

/// (Projesi belli toplantılar ve projeleri, hiçbir projeye düşmeyen toplantılar).
pub type SplitMeetings = (Vec<(Meeting, String)>, Vec<Meeting>);

/// Önceki sürümün tek zaman çizelgesinin taşındığı çizelgenin kimliği (her cihazda aynı).
pub fn first_timesheet_id() -> String {
    crate::classify::default_id("timesheet:first")
}

/// Kaydedilmiş zaman çizelgesi satırı.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEntry {
    pub id: String,
    #[serde(flatten)]
    pub entry: TimesheetEntry,
    /// Excel'e ya da Sheets'e aktarıldığı an; doluysa kayıt değiştirilemez.
    pub exported_at: Option<DateTime<Utc>>,
    /// Aktarıldığı zaman çizelgesi.
    pub timesheet_id: Option<String>,
    /// Gizlendi (silindi): gösterilmez, aktarılmaz; aralıkları yeniden önerilmez.
    pub dismissed: bool,
    /// Aktarılmış satırın dosyaya yazıldığı danışman adı (bilinmiyorsa çizelgeninki geçerli).
    pub consultant: Option<String>,
}

/// Günün bir zaman çizelgesindeki satırı: kaydedilmiş ya da takipten gelen (canlı) öneri.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayRow {
    /// Kaydedilmiş satırın kimliği; canlı öneride `None`.
    pub id: Option<String>,
    /// Sayfadaki kararlı anahtar: canlı öneri kaydedilince de aynı kalır.
    pub key: String,
    pub exported: bool,
    /// Kaydedildikten sonra işin bir kısmı raporda başka projeye (ya da projesize) alındıysa
    /// projede kalan gerçek süre (saat); satır güncellenmeden aktarılmaz.
    pub stale: Option<f64>,
    #[serde(flatten)]
    pub entry: TimesheetEntry,
}

/// Günün bir zaman çizelgesindeki satırları, başlangıca göre.
#[derive(Debug, Clone, PartialEq)]
pub struct DayRows {
    pub rows: Vec<DayRow>,
    /// Gizlenen satır sayısı.
    pub hidden: usize,
}

/// Günün zaman çizelgesine giren süresi ([`Store::timesheet_pieces`]).
#[derive(Debug, Clone, Default)]
pub struct TimesheetPieces {
    pub pieces: Vec<Piece>,
    /// Toplantısı yapılmamış sayılan (çizelgeden silinmiş) satırlar.
    skipped: HashSet<String>,
}

impl std::ops::Deref for TimesheetPieces {
    type Target = [Piece];

    fn deref(&self) -> &[Piece] {
        &self.pieces
    }
}

/// Satır toplantının satırı mı: aralıkları toplantının içinde ya da (aralıkları bilinmeyen eski
/// satırda) aynı gün ve saatte başlıyor; türü aynı.
fn meeting_row(m: &Meeting, e: &TimesheetEntry) -> bool {
    let kind = if m.online {
        EntryKind::Online
    } else {
        EntryKind::F2F
    };
    if e.kind != kind {
        return false;
    }
    let spans = e.spans();
    if e.coverage.is_some() {
        return !spans.is_empty() && spans.iter().all(|&(a, b)| m.start <= a && b <= m.end);
    }
    let start = m.start.with_timezone(&chrono::Local);
    e.date == start.date_naive()
        && e.start.format("%H:%M").to_string() == start.format("%H:%M").to_string()
}

/// Zaman çizelgesi hesabının ortak girdileri: birkaç gün boyunca bir kez okunur.
pub struct TimesheetContext {
    pub config: TimesheetConfig,
    classifier: Classifier,
    names: HashMap<String, String>,
    assigned: HashMap<String, Option<String>>,
}

impl TimesheetContext {
    /// Kaydedilmiş satırlar düşülmeden önerilenler (açıklamanın otomatik gelip gelmediğini
    /// anlamak için).
    pub fn proposals(&self, sheet: &Timesheet, pieces: &[Piece]) -> Vec<TimesheetEntry> {
        timesheet::propose(pieces, &self.names, sheet, &HashMap::new())
    }

    pub fn classifier(&self) -> &Classifier {
        &self.classifier
    }

    pub fn project_name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }
}

fn parse_kind(s: &str) -> Option<EntryKind> {
    match s {
        "Working" => Some(EntryKind::Working),
        "Online" => Some(EntryKind::Online),
        "F2F" => Some(EntryKind::F2F),
        _ => None,
    }
}

/// Satırın sayfadaki anahtarı: takipten gelen satırda proje ve ilk aralığın başı (kaydedilince
/// değişmez), diğerlerinde kimliği.
fn row_key(id: Option<&str>, e: &TimesheetEntry) -> String {
    match e.coverage.as_deref().and_then(<[_]>::first) {
        Some([start, _]) => format!("{}@{start}", e.project_id),
        None => id.unwrap_or_default().to_string(),
    }
}

fn overlaps(a: &[Interval], b: &[Interval]) -> bool {
    a.iter()
        .any(|&(x, y)| b.iter().any(|&(c, d)| x < d && c < y))
}

/// Önceki sürümün ayarı: tek dosya, bütün projeler.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LegacyConfig {
    company: String,
    consultant: String,
    file_path: Option<String>,
    sheet_url: Option<String>,
    sheet_link: Option<String>,
    sheet_token: String,
    default_party: String,
    projects: Vec<ProjectMapping>,
    meeting_apps: Option<Vec<String>>,
    day_hours: Option<f64>,
}

const COLUMNS: &str = "id, date, start, hours, kind, details, party, project_id, division,
    exported_at, actual_hours, coverage, timesheet_id, dismissed_at, consultant";

/// Satırın değiştiğini işaretler (eşitlemede gönderilsin): şimdiki an (ms), aynı milisaniyede
/// ikinci değişiklik de öncekinden yeni sayılsın diye en az bir fazlası.
const TOUCH: &str = "updated_at = MAX(updated_at + 1,
    CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))";

/// Satırın durumunun (aktarım, gizlenme, silinme, danışman) değiştiğini işaretler: eşitlemede
/// durum içerikten ayrı birleşir ([`crate::sync`]). `TOUCH` ile birlikte kullanılır.
const STATE: &str = "state_at = MAX(COALESCE(state_at, 0) + 1,
    CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))";

/// Takipten gelen satırın kimliği işinden türetilir (proje ve ilk aralığın başı): aynı öneri
/// iki bilgisayarda kaydedilirse tek satır olur. Elle eklenen satırınki rastgele.
fn new_entry_id(e: &TimesheetEntry) -> String {
    match e.coverage.as_deref().and_then(<[_]>::first) {
        Some(_) => Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("kum:timesheet:{}", row_key(None, e)).as_bytes(),
        )
        .to_string(),
        None => Uuid::new_v4().to_string(),
    }
}

impl Store {
    pub fn timesheet_config(&self) -> Result<TimesheetConfig> {
        Ok(self.setting(TIMESHEET_KEY)?.unwrap_or_default())
    }

    /// Ayarları kaydeder; bir proje yalnızca bir çizelgede kalır ([`TimesheetConfig::normalize`]).
    pub fn save_timesheet_config(&self, config: &TimesheetConfig) -> Result<()> {
        let mut config = config.clone();
        config.normalize();
        self.save_setting(TIMESHEET_KEY, &config)
    }

    /// Önceki sürümün tek zaman çizelgesini firmaya özel çizelgeye taşır (bir kez, açılışta).
    /// Çizelgeye eşlemesi olan (silinmemiş) projeler ve adı ya da müşterisinin adı firmayla
    /// aynı olan projeler girer; eşlemelerin birimleri çizelgenin birimleri olur. Aktarılmış
    /// satırlar bu çizelgeye aktarılmış sayılır. Birim seçilirken satırın projesi silinmiş bir
    /// birim projesine dönmüşse ve çizelgenin tek projesi varsa satır o projeye bağlanır;
    /// yoksa aynı iş yeni önerilerde ikinci kez gelirdi.
    pub(super) fn migrate_timesheets(&self) -> Result<()> {
        let Some(raw) = self.setting::<serde_json::Value>(TIMESHEET_KEY)? else {
            return Ok(());
        };
        if raw.get("timesheets").is_some() {
            return Ok(());
        }
        let old: LegacyConfig = serde_json::from_value(raw)?;
        let defaults = TimesheetConfig::default();
        let mut config = TimesheetConfig {
            timesheets: Vec::new(),
            sheet_token: old.sheet_token,
            meeting_apps: old.meeting_apps.unwrap_or(defaults.meeting_apps),
            day_hours: old.day_hours.unwrap_or(defaults.day_hours),
        };
        let company = old.company.trim().to_string();
        let used = old.file_path.is_some()
            || old.sheet_url.is_some()
            || !company.is_empty()
            || !old.projects.is_empty();
        let tx = self.savepoint()?;
        if used {
            let projects: Vec<crate::classify::Tag> = self
                .tags()?
                .into_iter()
                .filter(|t| t.kind == TagKind::Project)
                .collect();
            let exists = |id: &str| projects.iter().any(|t| t.id == id);
            let clients: HashMap<String, String> = self
                .clients()?
                .into_iter()
                .map(|c| (c.id, c.name))
                .collect();
            let project_clients = self.project_clients()?;
            let mut sheet = Timesheet {
                id: first_timesheet_id(),
                company: old.company,
                consultant: old.consultant,
                file_path: old.file_path,
                sheet_url: old.sheet_url,
                sheet_link: old.sheet_link,
                default_party: old.default_party,
                projects: Vec::new(),
                divisions: Vec::new(),
            };
            for m in old.projects {
                sheet.divisions.push(m.division.trim().to_string());
                if exists(&m.project_id) {
                    sheet.projects.push(m);
                }
            }
            // Birimi satırda seçilen tek projeyle çalışılıyordu (örn. "Togg" projesi).
            if !company.is_empty() {
                for t in &projects {
                    let client = project_clients
                        .get(&t.id)
                        .and_then(|c| clients.get(c))
                        .map(|c| c.trim());
                    let named = t.name.trim().eq_ignore_ascii_case(&company)
                        || client.is_some_and(|c| c.eq_ignore_ascii_case(&company));
                    if named && !sheet.includes(&t.id) {
                        sheet.projects.push(ProjectMapping {
                            project_id: t.id.clone(),
                            division: String::new(),
                            party: None,
                            default_details: None,
                        });
                    }
                }
            }
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET timesheet_id = ?1, {TOUCH}, {STATE}
                     WHERE exported_at IS NOT NULL AND timesheet_id IS NULL"
                ),
                [&sheet.id],
            )?;
            if let [only] = &sheet.projects[..] {
                let orphans: Vec<(String, String)> = self
                    .query_entries("1 = 1", [])?
                    .into_iter()
                    .filter(|s| !exists(&s.entry.project_id))
                    .map(|s| (s.id, s.entry.division))
                    .collect();
                for (id, division) in orphans {
                    let known = sheet
                        .divisions
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(division.trim()));
                    if known {
                        self.conn.execute(
                            &format!(
                                "UPDATE timesheet_entries SET project_id = ?2, {TOUCH}
                                 WHERE id = ?1"
                            ),
                            params![id, only.project_id],
                        )?;
                    }
                }
            }
            config.timesheets.push(sheet);
        }
        self.save_timesheet_config(&config)?;
        tx.commit()
    }

    /// Toplantı serilerinin elle verilen projeleri (UID → proje; `None`: yoksayıldı).
    pub fn meeting_assignments(&self) -> Result<HashMap<String, Option<String>>> {
        Ok(self.setting(MEETING_ASSIGNMENTS_KEY)?.unwrap_or_default())
    }

    /// Toplantı serisini projeye atar (`None`: zaman çizelgesine alma).
    pub fn assign_meeting(&self, uid: &str, project: Option<&str>) -> Result<()> {
        let mut all = self.meeting_assignments()?;
        all.insert(uid.to_string(), project.map(str::to_string));
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)
    }

    /// Yoksayılan toplantıları geri getirir; sayısını döndürür.
    pub fn restore_ignored_meetings(&self) -> Result<usize> {
        let mut all = self.meeting_assignments()?;
        let before = all.len();
        all.retain(|_, p| p.is_some());
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)?;
        Ok(before - all.len())
    }

    /// Toplantıları projesine göre ayırır: (projesi belli olanlar, hiçbir projeye düşmeyenler).
    pub fn classify_meetings(&self, meetings: &[Meeting]) -> Result<SplitMeetings> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let assigned = self.meeting_assignments()?;
        let (mut known, mut unassigned) = (Vec::new(), Vec::new());
        for m in meetings {
            match timesheet::meeting_project(m, &classifier, &assigned) {
                MeetingProject::Project(p) => known.push((m.clone(), p)),
                MeetingProject::Unassigned => unassigned.push(m.clone()),
                MeetingProject::Ignored => {}
            }
        }
        Ok((known, unassigned))
    }

    /// Toplantı → proje öneri modeli ([`crate::meeting_suggest`]). `series` takvimdeki her
    /// seriden bir örnektir ([`crate::calendar::Calendar::series`]); projesi belli olanlar
    /// (elle atanan ya da kurala uyan) geçmiş olur. Müşterinin projeleri arasında son
    /// [`SUGGEST_USAGE_DAYS`] günün zaman çizelgesi saatlerine göre seçilir.
    pub fn meeting_suggester(&self, series: &[Meeting]) -> Result<MeetingSuggester> {
        let tags = self.tags()?;
        let rules = self.rules()?;
        let classifier = Classifier::new(&tags, &rules);
        let assigned = self.meeting_assignments()?;
        let history: Vec<(Meeting, String)> = series
            .iter()
            .filter_map(
                |m| match timesheet::meeting_project(m, &classifier, &assigned) {
                    MeetingProject::Project(p) => Some((m.clone(), p)),
                    _ => None,
                },
            )
            .collect();
        let client_names: HashMap<String, String> = self
            .clients()?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let project_clients = self.project_clients()?;
        let archived = self.archived_projects()?;
        let projects: Vec<ProjectInfo> = tags
            .into_iter()
            .filter(|t| t.kind == TagKind::Project)
            .map(|t| ProjectInfo {
                client: project_clients
                    .get(&t.id)
                    .and_then(|c| client_names.get(c))
                    .cloned(),
                archived: archived.contains(&t.id),
                id: t.id,
                name: t.name,
            })
            .collect();
        let today = chrono::Local::now().date_naive();
        let mut usage: HashMap<String, f64> = HashMap::new();
        for e in self.timesheet_entries(today - chrono::Days::new(SUGGEST_USAGE_DAYS), today)? {
            *usage.entry(e.entry.project_id).or_default() += e.entry.hours;
        }
        Ok(MeetingSuggester::new(&SuggestInput {
            projects: &projects,
            history: &history,
            all: series,
            rules: &rules,
            usage: &usage,
        }))
    }

    /// Zaman çizelgesi hesabının ortak girdileri (ayarlar, sınıflandırma, toplantı atamaları).
    pub fn timesheet_context(&self) -> Result<TimesheetContext> {
        let tags = self.tags()?;
        Ok(TimesheetContext {
            classifier: Classifier::new(&tags, &self.rules()?),
            names: tags
                .into_iter()
                .filter(|t| t.kind == TagKind::Project)
                .map(|t| (t.id, t.name))
                .collect(),
            config: self.timesheet_config()?,
            assigned: self.meeting_assignments()?,
        })
    }

    /// `from`–`to` (yerel gün) arasında zaman çizelgesine giren süre: projesi belli takvim
    /// toplantıları ve oturumlar ([`timesheet::pieces`]). Çizelgeden silinmiş satırı olan
    /// toplantı yapılmamış sayılır: süresi çakışan başka toplantıya ya da o saatteki işe kalır.
    pub fn timesheet_pieces(
        &self,
        ctx: &TimesheetContext,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<TimesheetPieces> {
        let day = |t: DateTime<Utc>| t.with_timezone(&chrono::Local).date_naive();
        let (dismissed, kept): (Vec<SavedEntry>, Vec<SavedEntry>) = self
            .saved_rows(day(from), day(to))?
            .into_iter()
            .filter(|s| s.entry.kind != EntryKind::Working)
            .partition(|s| s.dismissed);
        let mut skipped = HashSet::new();
        let known: Vec<(Meeting, String)> = meetings
            .iter()
            .filter_map(
                |m| match timesheet::meeting_project(m, &ctx.classifier, &ctx.assigned) {
                    MeetingProject::Project(p) => Some((m.clone(), p)),
                    _ => None,
                },
            )
            .filter(|(m, p)| {
                let of_meeting =
                    |s: &&SavedEntry| s.entry.project_id == *p && meeting_row(m, &s.entry);
                // Toplantının bir parçası çizelgede duruyorsa (ör. uzayan toplantının artığı
                // silindi) toplantı yapılmıştır.
                if kept.iter().any(|s| of_meeting(&s)) {
                    return true;
                }
                let rows: Vec<&SavedEntry> = dismissed.iter().filter(of_meeting).collect();
                skipped.extend(rows.iter().map(|s| s.id.clone()));
                rows.is_empty()
            })
            .collect();
        let sessions = self.merged_sessions_between(from, to)?;
        Ok(TimesheetPieces {
            pieces: timesheet::pieces(&sessions, &known, &ctx.classifier, &ctx.config, from, to),
            skipped,
        })
    }

    /// Günün `sheet` çizelgesindeki satırları: kaydedilmiş satırlar (aktarılmışsa bu çizelgeye
    /// aktarılanlar, değilse çizelgenin projelerininkiler) ve kaydedilmiş satırların aralıkları
    /// düşülmüş canlı öneriler. `pieces` günün parçalarıdır ([`Self::timesheet_pieces`]).
    pub fn timesheet_day(
        &self,
        ctx: &TimesheetContext,
        sheet: &Timesheet,
        date: NaiveDate,
        pieces: &TimesheetPieces,
    ) -> Result<DayRows> {
        let saved = self.saved_rows(date, date)?;
        let mut covered: HashMap<String, Vec<Vec<Interval>>> = HashMap::new();
        let mut legacy = Vec::new();
        // Yapılmamış sayılan toplantının silinmiş satırı başka işi kapsamaz.
        for s in saved.iter().filter(|s| !pieces.skipped.contains(&s.id)) {
            match &s.entry.coverage {
                Some(_) => covered
                    .entry(s.entry.project_id.clone())
                    .or_default()
                    .push(s.entry.spans()),
                None if sheet.includes(&s.entry.project_id) => legacy.push(s.entry.clone()),
                None => {}
            }
        }
        let live = timesheet::without_legacy(
            &timesheet::propose(pieces, &ctx.names, sheet, &covered),
            &legacy,
        );
        let mut rows = Vec::new();
        let mut hidden = 0;
        for s in saved {
            let routed = match (&s.exported_at, &s.timesheet_id) {
                (Some(_), Some(t)) => *t == sheet.id,
                _ => sheet.includes(&s.entry.project_id),
            };
            if !routed {
                continue;
            }
            if s.dismissed {
                hidden += 1;
                continue;
            }
            // Aktarılmış satır da takipte değişebilir: güncellenince dosyadaki satırı da değişir.
            let stale = timesheet::stale_hours(pieces, &s.entry);
            rows.push(DayRow {
                key: row_key(Some(&s.id), &s.entry),
                id: Some(s.id),
                exported: s.exported_at.is_some(),
                stale,
                entry: s.entry,
            });
        }
        rows.extend(live.into_iter().map(|entry| DayRow {
            key: row_key(None, &entry),
            id: None,
            exported: false,
            stale: None,
            entry,
        }));
        rows.sort_by(|a, b| {
            a.entry
                .start
                .cmp(&b.entry.start)
                .then(a.entry.division.cmp(&b.entry.division))
        });
        Ok(DayRows { rows, hidden })
    }

    /// `days` günlerinde (`day_starts`: yerel gün sınırları, `days.len() + 1` öğe) bütün zaman
    /// çizelgelerinin satırları: kaydedilmiş ve canlı, gizlenenler hariç. Artık olmayan bir
    /// çizelgeye aktarılmış satırlar da girer (gönderilmiş iş).
    pub fn timesheet_rows(
        &self,
        days: &[NaiveDate],
        day_starts: &[DateTime<Utc>],
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        let ctx = self.timesheet_context()?;
        let mut out = Vec::new();
        for (date, bounds) in days.iter().zip(day_starts.windows(2)) {
            let (from, to) = (bounds[0], bounds[1]);
            if !ctx.config.timesheets.is_empty() {
                let todays: Vec<Meeting> = meetings
                    .iter()
                    .filter(|m| m.start < to && m.end > from)
                    .cloned()
                    .collect();
                let pieces = self.timesheet_pieces(&ctx, from, to, &todays)?;
                for sheet in &ctx.config.timesheets {
                    let day = self.timesheet_day(&ctx, sheet, *date, &pieces)?;
                    out.extend(day.rows.into_iter().map(|r| r.entry));
                }
            }
            out.extend(
                self.saved_rows(*date, *date)?
                    .into_iter()
                    .filter(|s| {
                        !s.dismissed
                            && s.exported_at.is_some()
                            && s.timesheet_id
                                .as_deref()
                                .is_some_and(|t| ctx.config.timesheet(t).is_none())
                    })
                    .map(|s| s.entry),
            );
        }
        Ok(out)
    }

    /// `[from, to]` tarihleri (dahil) arasındaki kaydedilmiş, gizlenmemiş satırlar (bütün
    /// çizelgeler), tarih ve saate göre.
    pub fn timesheet_entries(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<SavedEntry>> {
        let mut rows = self.saved_rows(from, to)?;
        rows.retain(|s| !s.dismissed);
        Ok(rows)
    }

    /// Kaydedilmiş satır (gizlenmiş de olabilir).
    pub fn timesheet_entry(&self, id: &str) -> Result<Option<SavedEntry>> {
        Ok(self.query_entries("id = ?1", params![id])?.pop())
    }

    /// Gizlenenler dahil bütün satırlar.
    fn saved_rows(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<SavedEntry>> {
        self.query_entries(
            "date >= ?1 AND date <= ?2",
            params![from.to_string(), to.to_string()],
        )
    }

    fn query_entries(&self, filter: &str, args: impl rusqlite::Params) -> Result<Vec<SavedEntry>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM timesheet_entries WHERE deleted_at IS NULL AND ({filter})
             ORDER BY date, start"
        ))?;
        let rows = stmt.query_map(args, |r| {
            Ok((
                (
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, String>(4)?,
                ),
                (
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ),
                (
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, Option<f64>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                    r.get::<_, Option<i64>>(13)?,
                    r.get::<_, Option<String>>(14)?,
                ),
            ))
        })?;
        rows.map(|row| {
            let (
                (id, date, start, hours, kind),
                (details, party, project_id, division),
                (exported, actual, coverage, timesheet_id, dismissed, consultant),
            ) = row?;
            let bad = |what: &str| StoreError::Invalid(format!("zaman çizelgesi {what}: {id}"));
            Ok(SavedEntry {
                entry: TimesheetEntry {
                    date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                        .map_err(|_| bad("tarihi"))?,
                    start: NaiveTime::parse_from_str(&start, "%H:%M").map_err(|_| bad("saati"))?,
                    hours,
                    actual_hours: actual,
                    kind: parse_kind(&kind).ok_or_else(|| bad("türü"))?,
                    details,
                    party,
                    project_id,
                    division,
                    coverage: coverage
                        .map(|c| serde_json::from_str(&c))
                        .transpose()
                        .map_err(|_| bad("aralıkları"))?,
                },
                exported_at: exported.map(from_ms),
                timesheet_id,
                dismissed: dismissed.is_some(),
                consultant,
                id,
            })
        })
        .collect()
    }

    /// Satırı kaydeder. Kaydedilmiş satır güncellenir (aktarılmışsa değiştirilemez; aralıkları
    /// değişmez). Yeni satır eklenir: canlı öneri aralıklarıyla, elle eklenen satır aralıksız.
    /// Öneri ikinci kez kaydedilirse (arayüz eski listeyle) aynı satır güncellenir; aralıkları
    /// başka bir satırla kesişiyorsa reddedilir (aynı iş iki kez yazılmasın).
    pub fn save_timesheet_entry(&self, id: Option<&str>, entry: &TimesheetEntry) -> Result<String> {
        if !(entry.hours > 0.0 && entry.hours <= 24.0) {
            return Err(StoreError::Invalid("saat 0 ile 24 arasında olmalı".into()));
        }
        let tx = self.savepoint()?;
        let existing = match id {
            // Başka cihazda silinmiş (ya da birleşmiş) satır eski sayfadan düzenlenince geri
            // canlanmasın: silme diğer cihaza geri yayılır, iş iki kez sayılırdı.
            Some(id) => Some(self.timesheet_entry(id)?.ok_or_else(|| {
                StoreError::Invalid("Satır değişti; sayfa yenilendi, tekrar dene.".into())
            })?),
            None => None,
        };
        let id = match existing {
            Some(saved) => {
                if saved.exported_at.is_some() {
                    return Err(StoreError::Invalid(
                        "Aktarılmış kayıt değiştirilemez".into(),
                    ));
                }
                self.update_entry(&saved.id, entry)?;
                saved.id
            }
            None => {
                let coverage = entry.coverage.clone().unwrap_or_default();
                match self.claim_spans(entry, &coverage)? {
                    Some(same) => {
                        self.update_entry(&same, entry)?;
                        same
                    }
                    None => {
                        let id = id.map_or_else(|| new_entry_id(entry), str::to_string);
                        self.insert_entry(
                            &id,
                            &TimesheetEntry {
                                coverage: Some(coverage),
                                ..entry.clone()
                            },
                        )?;
                        id
                    }
                }
            }
        };
        tx.commit()?;
        Ok(id)
    }

    /// Canlı öneriyle aynı aralıkları kapsayan kaydedilmiş (aktarılmamış, gizlenmemiş) satır:
    /// arayüz satırı kaydetmiş ama listesi henüz yenilenmemişse öneri aslında bu satırdır.
    pub fn saved_twin(&self, entry: &TimesheetEntry) -> Result<Option<SavedEntry>> {
        let Some(coverage) = entry.coverage.as_deref().filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        let spans = timesheet::from_coverage(coverage);
        Ok(self
            .saved_rows(entry.date, entry.date)?
            .into_iter()
            .find(|s| {
                s.entry.project_id == entry.project_id
                    && s.exported_at.is_none()
                    && !s.dismissed
                    && s.entry.spans() == spans
            }))
    }

    /// Takipten gelen satır kaydedilirken: aynı aralıklar zaten kaydedildiyse o satırın kimliği;
    /// aralıklar başka bir satırınkilerle kesişiyorsa hata.
    fn claim_spans(&self, entry: &TimesheetEntry, coverage: &[[i64; 2]]) -> Result<Option<String>> {
        if coverage.is_empty() {
            return Ok(None);
        }
        let spans = timesheet::from_coverage(coverage);
        for s in self.saved_rows(entry.date, entry.date)? {
            if s.entry.project_id != entry.project_id {
                continue;
            }
            let theirs = s.entry.spans();
            if !overlaps(&spans, &theirs) {
                continue;
            }
            if theirs == spans && s.exported_at.is_none() && !s.dismissed {
                return Ok(Some(s.id));
            }
            return Err(StoreError::Invalid(
                "Bu satırın işi zaten kaydedilmiş; sayfa yenilendi, tekrar dene.".into(),
            ));
        }
        Ok(None)
    }

    /// Satırı gizler (siler): gösterilmez, aktarılmaz; aralıkları yeniden önerilmez. Canlı
    /// öneri (`id` yok) gizlenmiş olarak kaydedilir. Satırın kimliğini döndürür.
    pub fn dismiss_timesheet_entry(
        &self,
        id: Option<&str>,
        entry: &TimesheetEntry,
    ) -> Result<String> {
        let tx = self.savepoint()?;
        let id = match id {
            Some(id) => id.to_string(),
            None => self.save_timesheet_entry(None, entry)?,
        };
        let n = self.conn.execute(
            &format!(
                "UPDATE timesheet_entries SET dismissed_at = ?2, {TOUCH}, {STATE}
                 WHERE id = ?1 AND exported_at IS NULL AND deleted_at IS NULL"
            ),
            params![id, ms(Utc::now())],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid(
                "Satır bulunamadı ya da aktarılmış; silinemez.".into(),
            ));
        }
        tx.commit()?;
        Ok(id)
    }

    /// `[from, to)` raporda `project`'e atandı: projenin bu aralığı kaplayan silinmiş satırları
    /// aralığı artık kapsamaz (iş yeniden canlı öneri olur); aralığı kalmayan silinmiş satır
    /// tamamen silinir. Değişen satır sayısı.
    pub(crate) fn forget_dismissed(
        &self,
        project: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<usize> {
        let day = |t: DateTime<Utc>| t.with_timezone(&chrono::Local).date_naive();
        let mut n = 0;
        for s in self.saved_rows(day(from), day(to))? {
            if !s.dismissed || s.entry.project_id != project || s.entry.coverage.is_none() {
                continue;
            }
            let spans = s.entry.spans();
            let left: Vec<Interval> = spans
                .iter()
                .flat_map(|&span| timesheet::subtract(vec![span], (from, to)))
                .collect();
            if left == spans {
                continue;
            }
            n += if left.is_empty() {
                self.remove_entry(&s.id, false)?
            } else {
                let coverage = serde_json::to_string(&timesheet::to_coverage(&left))?;
                self.set_coverage(&s.id, Some(coverage))?
            };
        }
        Ok(n)
    }

    /// Gizlenen satırları geri getirir.
    pub fn undismiss_timesheet_entries(&self, ids: &[String]) -> Result<()> {
        let tx = self.savepoint()?;
        for id in ids {
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET dismissed_at = NULL, {TOUCH}, {STATE}
                     WHERE id = ?1 AND dismissed_at IS NOT NULL AND deleted_at IS NULL"
                ),
                [id],
            )?;
        }
        tx.commit()
    }

    /// Günün `sheet` çizelgesinde gizlenen satırlarını geri getirir; kimliklerini döndürür.
    pub fn restore_hidden(&self, sheet: &Timesheet, date: NaiveDate) -> Result<Vec<String>> {
        let ids: Vec<String> = self
            .saved_rows(date, date)?
            .into_iter()
            .filter(|s| s.dismissed && sheet.includes(&s.entry.project_id))
            .map(|s| s.id)
            .collect();
        self.undismiss_timesheet_entries(&ids)?;
        Ok(ids)
    }

    /// Satırı tamamen siler (aktarılmamışsa). Canlı öneriyi gizlemek bunu geri alır: öneri yeniden gelir.
    pub fn delete_timesheet_entry(&self, id: &str) -> Result<()> {
        self.remove_entry(id, true)?;
        Ok(())
    }

    /// Satırları birleştirir ([`timesheet::merge`]): kaydedilmiş satırlar (`Some(kimlik)`)
    /// silinir, canlı öneriler (`None`) aralıklarıyla katılır, sonuç yeni satır olur. Yeni
    /// satırın kimliği ve geri almak için silinen satırlar döner.
    #[allow(clippy::type_complexity)]
    pub fn merge_timesheet_entries(
        &self,
        rows: &[(Option<String>, TimesheetEntry)],
    ) -> Result<(String, Vec<(String, TimesheetEntry)>)> {
        let tx = self.savepoint()?;
        let mut entries = Vec::with_capacity(rows.len());
        let mut removed = Vec::new();
        for (id, entry) in rows {
            let Some(id) = id else {
                entries.push(entry.clone());
                continue;
            };
            let saved = self
                .timesheet_entry(id)?
                .filter(|s| !s.dismissed)
                .ok_or_else(|| {
                    StoreError::Invalid("Satır bulunamadı; sayfa yenilendi, tekrar dene.".into())
                })?;
            if saved.exported_at.is_some() {
                return Err(StoreError::Invalid(
                    "Aktarılmış satır birleştirilemez".into(),
                ));
            }
            entries.push(saved.entry.clone());
            removed.push((saved.id, saved.entry));
        }
        let merged = timesheet::merge(&entries).map_err(|e| StoreError::Invalid(e.to_string()))?;
        for (id, _) in &removed {
            self.remove_entry(id, false)?;
        }
        let coverage = merged.coverage.clone().unwrap_or_default();
        if self.claim_spans(&merged, &coverage)?.is_some() {
            return Err(StoreError::Invalid(
                "Bu satırın işi zaten kaydedilmiş; sayfa yenilendi, tekrar dene.".into(),
            ));
        }
        let id = Uuid::new_v4().to_string();
        self.insert_entry(&id, &merged)?;
        tx.commit()?;
        Ok((id, removed))
    }

    /// Birleştirmeyi geri alır: birleşen satır silinir, önceki satırlar aynen geri gelir.
    pub fn unmerge_timesheet_entries(
        &self,
        merged: &str,
        removed: &[(String, TimesheetEntry)],
    ) -> Result<()> {
        let tx = self.savepoint()?;
        let n = self.remove_entry(merged, true)?;
        if n == 0 {
            return Err(StoreError::Invalid(
                "Birleşen satır aktarılmış ya da silinmiş; geri alınamaz.".into(),
            ));
        }
        for (id, entry) in removed {
            self.insert_entry(id, entry)?;
        }
        tx.commit()
    }

    /// Takipte değişen satırı günceller ([`timesheet::refreshed`]): aralıkları projede kalan
    /// süreye iner; hiç süre kalmadıysa satır silinir (`false`). Aktarılmış, gizlenmiş, elle
    /// eklenen ve eski satırlara dokunulmaz.
    pub fn refresh_timesheet_entry(&self, id: &str, pieces: &[Piece]) -> Result<bool> {
        let Some(saved) = self.timesheet_entry(id)? else {
            return Ok(false);
        };
        if saved.exported_at.is_some() || saved.dismissed || saved.entry.spans().is_empty() {
            return Ok(true);
        }
        match timesheet::refreshed(pieces, &saved.entry) {
            Some(fresh) => {
                let tx = self.savepoint()?;
                self.update_entry(id, &fresh)?;
                self.set_coverage(id, coverage_json(&fresh)?)?;
                tx.commit()?;
                Ok(true)
            }
            None => {
                self.delete_timesheet_entry(id)?;
                Ok(false)
            }
        }
    }

    /// "Yeniden öner": günün `sheet` çizelgesindeki aktarılmamış satırları (gizlenenler dahil)
    /// siler; iş yeniden canlı öneri olur. Silinen satır sayısı.
    pub fn reset_timesheet_day(&self, sheet: &Timesheet, date: NaiveDate) -> Result<usize> {
        let tx = self.savepoint()?;
        let mut n = 0;
        for s in self.saved_rows(date, date)? {
            if s.exported_at.is_none() && sheet.includes(&s.entry.project_id) {
                n += self.remove_entry(&s.id, false)?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Aktarılmış satırı dosyadaki satırıyla birlikte değiştirir (dosyaya yazıldıktan sonra):
    /// düzenlenebilir alanlar, `coverage` ise aralıkları da (takipte değişen satır güncellenince).
    /// Satır aktarılmamışsa ya da gizlenmişse hata.
    /// `consultant`: satır dosyaya bu danışman adıyla yazıldı (ya da dosyada öyle okundu).
    pub fn save_exported_entry(
        &self,
        id: &str,
        entry: &TimesheetEntry,
        coverage: bool,
        consultant: Option<&str>,
    ) -> Result<()> {
        if !(entry.hours > 0.0 && entry.hours <= 24.0) {
            return Err(StoreError::Invalid("saat 0 ile 24 arasında olmalı".into()));
        }
        let saved = self
            .timesheet_entry(id)?
            .filter(|s| s.exported_at.is_some() && !s.dismissed)
            .ok_or_else(|| StoreError::Invalid("Aktarılmış satır bulunamadı.".into()))?;
        let tx = self.savepoint()?;
        self.update_entry(&saved.id, entry)?;
        if coverage {
            self.set_coverage(id, coverage_json(entry)?)?;
        }
        if let Some(c) = consultant.map(str::trim).filter(|c| !c.is_empty())
            && saved.consultant.as_deref() != Some(c)
        {
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET consultant = ?2, {TOUCH}, {STATE} WHERE id = ?1"
                ),
                params![id, c],
            )?;
        }
        tx.commit()
    }

    /// Aktarılmış satır dosyadan kaldırıldı: satır aktarılmamış olur ve gizlenir (`dismiss`;
    /// aralıkları yeniden önerilmez, "geri getir" ile yeniden gönderilebilir) ya da silinir
    /// (işi projede kalmamış satır).
    pub fn withdraw_exported_entry(&self, id: &str, dismiss: bool) -> Result<()> {
        let tx = self.savepoint()?;
        if dismiss {
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET exported_at = NULL, timesheet_id = NULL,
                        consultant = NULL, dismissed_at = ?2, {TOUCH}, {STATE} WHERE id = ?1"
                ),
                params![id, ms(Utc::now())],
            )?;
        } else {
            self.remove_entry(id, false)?;
        }
        tx.commit()
    }

    /// Aktarılan kayıtları `sheet_id` çizelgesine `consultant` danışman adıyla aktarılmış
    /// işaretler.
    pub fn mark_timesheet_exported(
        &self,
        ids: &[String],
        at: DateTime<Utc>,
        sheet_id: &str,
        consultant: &str,
    ) -> Result<()> {
        let tx = self.savepoint()?;
        for id in ids {
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET exported_at = ?2, timesheet_id = ?3,
                        consultant = NULLIF(TRIM(?4), ''), {TOUCH}, {STATE}
                     WHERE id = ?1"
                ),
                params![id, ms(at), sheet_id, consultant],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Aktarım geri alınınca kayıtlar yeniden aktarılmamış sayılır.
    pub fn unmark_timesheet_exported(&self, ids: &[String]) -> Result<()> {
        let tx = self.savepoint()?;
        for id in ids {
            self.conn.execute(
                &format!(
                    "UPDATE timesheet_entries SET exported_at = NULL, timesheet_id = NULL,
                        consultant = NULL, {TOUCH}, {STATE}
                     WHERE id = ?1"
                ),
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Satırın düzenlenebilir alanlarını yazar (aralıkları, aktarım ve gizlenme durumu değişmez).
    fn update_entry(&self, id: &str, e: &TimesheetEntry) -> Result<()> {
        self.conn.execute(
            &format!(
                "UPDATE timesheet_entries SET date = ?2, start = ?3, hours = ?4, kind = ?5,
                    details = ?6, party = ?7, project_id = ?8, division = ?9, actual_hours = ?10,
                    {TOUCH}
                 WHERE id = ?1"
            ),
            params![
                id,
                e.date.to_string(),
                e.start.format("%H:%M").to_string(),
                e.hours,
                e.kind.label(),
                e.details.trim(),
                e.party.trim(),
                e.project_id,
                e.division.trim(),
                e.actual_hours,
            ],
        )?;
        Ok(())
    }

    /// Yeni satır yazar. Aynı kimlikte silinmiş satır varsa (birleştirme geri alınınca) o satır
    /// yeniden canlanır: aktarılmamış, gizlenmemiş, yeni değerlerle.
    fn insert_entry(&self, id: &str, e: &TimesheetEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO timesheet_entries
                (id, date, start, hours, kind, details, party, project_id, division, created_at,
                 actual_hours, coverage, updated_at, state_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?10, ?10)
             ON CONFLICT (id) DO UPDATE SET date = excluded.date, start = excluded.start,
                hours = excluded.hours, kind = excluded.kind, details = excluded.details,
                party = excluded.party, project_id = excluded.project_id,
                division = excluded.division, actual_hours = excluded.actual_hours,
                coverage = excluded.coverage, exported_at = NULL, timesheet_id = NULL,
                dismissed_at = NULL, deleted_at = NULL, consultant = NULL,
                updated_at = MAX(timesheet_entries.updated_at + 1, excluded.updated_at),
                state_at = MAX(COALESCE(timesheet_entries.state_at, 0) + 1, excluded.state_at)",
            params![
                id,
                e.date.to_string(),
                e.start.format("%H:%M").to_string(),
                e.hours,
                e.kind.label(),
                e.details.trim(),
                e.party.trim(),
                e.project_id,
                e.division.trim(),
                ms(Utc::now()),
                e.actual_hours,
                coverage_json(e)?,
            ],
        )?;
        Ok(())
    }
}

impl Store {
    /// Satırı siler (`unexported`: yalnızca aktarılmamışsa). Silme yumuşaktır (`deleted_at`):
    /// diğer cihazlara da ulaşır; okumalar silinen satırı görmez. Silinen satır sayısı.
    fn remove_entry(&self, id: &str, unexported: bool) -> Result<usize> {
        let only = if unexported {
            " AND exported_at IS NULL"
        } else {
            ""
        };
        Ok(self.conn.execute(
            &format!(
                "UPDATE timesheet_entries SET deleted_at = ?2, {TOUCH}, {STATE}
                 WHERE id = ?1 AND deleted_at IS NULL{only}"
            ),
            params![id, ms(Utc::now())],
        )?)
    }

    fn set_coverage(&self, id: &str, coverage: Option<String>) -> Result<usize> {
        Ok(self.conn.execute(
            &format!("UPDATE timesheet_entries SET coverage = ?2, {TOUCH} WHERE id = ?1"),
            params![id, coverage],
        )?)
    }
}

/// Satırın aralıkları veritabanındaki biçimiyle (JSON; eski satırda `NULL`).
fn coverage_json(e: &TimesheetEntry) -> Result<Option<String>> {
    Ok(e.coverage.as_ref().map(serde_json::to_string).transpose()?)
}

#[cfg(test)]
mod tests;
