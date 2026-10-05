//! Zaman çizelgesi komutları: günlük kayıt önerileri (takip edilen süre ve takvim
//! toplantıları), onaylama ve düzenleme, şablonu içe aktarma ve kayıtları kullanıcının
//! Excel dosyasına ya da Google Sheets tablosuna ekleme.

use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Days, NaiveDate, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::meeting_suggest::{MeetingSuggester, MeetingSuggestion};
use tracky_core::timesheet::{Meeting, ProjectMapping, TimesheetConfig, TimesheetEntry};
use tracky_core::{Rule, RuleField, Store, Tag, TagKind};

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Excel'e aktarım sürüyor: ikinci bir aktarım (çift tıklama) aynı kayıtları dosyaya
/// ikinci kez yazmasın diye reddedilir.
static EXPORTING: AtomicBool = AtomicBool::new(false);
/// Geri alınabilecek son aktarım (yalnızca bu oturumda).
static LAST_EXPORT: std::sync::Mutex<Option<LastExport>> = std::sync::Mutex::new(None);

struct LastExport {
    ids: Vec<String>,
    target: ExportTarget,
}

enum ExportTarget {
    Sheets {
        url: String,
        token: String,
    },
    /// Dosya aktarımdan sonra değiştiyse yedek geri yüklenmez (sonraki emek kaybolmasın).
    Excel {
        path: std::path::PathBuf,
        backup: std::path::PathBuf,
        modified: Option<std::time::SystemTime>,
    },
}

fn modified(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Aktarım bayrağını bırakır (hata ya da panikte de).
struct ExportGuard;

impl Drop for ExportGuard {
    fn drop(&mut self) {
        EXPORTING.store(false, Ordering::Release);
    }
}

/// Aktarım sürerken kayıtları değiştiren komutları reddeder: aktarım, yazdığı kayıtları sonunda
/// "aktarıldı" işaretler; arada değişen kayıt dosyaya eski haliyle yazılmış ya da hiç
/// yazılmamışken işaretlenirdi. Depo kilidi tutulurken çağrılmalı (aktarım bekleyen kayıtları
/// aynı kilit altında okur).
fn not_exporting() -> CmdResult<()> {
    if EXPORTING.load(Ordering::Acquire) {
        return Err("Zaman çizelgesi aktarılıyor; aktarım bitince tekrar dene.".into());
    }
    Ok(())
}

/// Geçmiş açıklamalar (otomatik tamamlama), şablondan içe aktarılır.
const DETAILS_KEY: &str = "timesheet_details";
const MAX_DETAILS: usize = 300;
const NO_TARGET: &str =
    "Önce Excel dosyasını seç ya da Google Sheets'e bağlan (Zaman çizelgesi ayarları)";

/// Bir günün kayıtları: onaylandıysa kaydedilenler, yoksa canlı öneri.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    date: NaiveDate,
    approved: bool,
    entries: Vec<EntryView>,
    /// Bir projeye atanmamış takip edilen süre (saniye): gözden geçirilecek.
    unassigned_seconds: i64,
    /// Takvimde olup hiçbir projeye düşmeyen toplantılar: projeye ata ya da yoksay.
    meetings: Vec<UnassignedMeeting>,
}

/// Projesi belli olmayan toplantı ve (eminse) önerilen proje.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedMeeting {
    #[serde(flatten)]
    meeting: Meeting,
    suggestion: Option<MeetingSuggestion>,
}

/// Öneri modeli yalnızca projesiz toplantı varsa ve bir kez kurulur (depo kilidi tutulurken).
struct LazySuggester<'a> {
    series: &'a [Meeting],
    model: Option<MeetingSuggester>,
}

impl LazySuggester<'_> {
    fn suggest(&mut self, store: &Store, meeting: &Meeting) -> Option<MeetingSuggestion> {
        if self.model.is_none() {
            // Kurulamazsa (depo hatası) öneri yok; liste yine gösterilir.
            self.model = Some(store.meeting_suggester(self.series).unwrap_or_default());
        }
        self.model.as_ref()?.suggest(meeting)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    /// Onaylanmış kaydın kimliği; öneride boş.
    id: Option<String>,
    exported: bool,
    #[serde(flatten)]
    entry: TimesheetEntry,
}

fn parse_date(s: &str) -> CmdResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("geçersiz tarih {s}: {e}"))
}

#[tauri::command]
pub async fn get_timesheet_config(app: AppHandle) -> CmdResult<TimesheetConfig> {
    lock(&app.state::<Shared>().store)
        .timesheet_config()
        .map_err(err)
}

#[tauri::command]
pub async fn save_timesheet_config(app: AppHandle, mut config: TimesheetConfig) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    // Apps Script anahtarını yalnızca arka uç üretir; arayüzün elindeki kopya, anahtar
    // üretilmeden önce yüklenmiş olabilir. Eski kopya yeni anahtarı ezmesin.
    config.sheet_token = store.timesheet_config().map_err(err)?.sheet_token;
    store.save_timesheet_config(&config).map_err(err)
}

/// `from`–`to` (dahil) arasında zaman çizelgesine henüz aktarılmamış işi olan günler: onaylanıp
/// aktarılmamış kaydı ya da aktarılmamış önerisi (takip edilen süre, toplantı) olan günler.
/// Kayıtların yazılacağı yer (Excel ya da Sheets) seçilmemişse boştur.
pub fn unexported_days(app: &AppHandle, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let meetings =
        crate::calendar::meetings(app, local_midnight(from), local_midnight(to + Days::new(1)));
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let configured = store
        .timesheet_config()
        .is_ok_and(|c| c.file_path.is_some() || c.sheet_url.is_some());
    if !configured {
        return Vec::new();
    }
    let pending = |date: NaiveDate| -> Result<bool, tracky_core::StoreError> {
        let saved = store.timesheet_entries(date, date)?;
        if saved.iter().any(|e| e.exported_at.is_none()) {
            return Ok(true);
        }
        let (start, end) = (local_midnight(date), local_midnight(date + Days::new(1)));
        let todays: Vec<Meeting> = meetings
            .iter()
            .filter(|m| m.start < end && m.end > start)
            .cloned()
            .collect();
        let proposed = store.propose_timesheet(start, end, &todays)?;
        let exported: Vec<TimesheetEntry> = saved.into_iter().map(|e| e.entry).collect();
        Ok(!tracky_core::timesheet::without_exported(&proposed, &exported).is_empty())
    };
    from.iter_days()
        .take_while(|d| *d <= to)
        .filter(|d| pending(*d).unwrap_or(false))
        .collect()
}

/// Bu hafta (pazartesiden bugüne) zaman çizelgesine aktarılmamış işi olan günler
/// (kenar çubuğu rozeti; kayıtların yazılacağı yer seçilmemişse boş).
#[tauri::command]
pub async fn pending_timesheet_days(app: AppHandle) -> CmdResult<Vec<NaiveDate>> {
    let today = chrono::Local::now().date_naive();
    let week = today
        - Days::new(u64::from(
            chrono::Datelike::weekday(&today).num_days_from_monday(),
        ));
    Ok(unexported_days(&app, week, today))
}

/// `start` gününden itibaren `days` günün kayıtları.
#[tauri::command]
pub async fn timesheet_days(app: AppHandle, start: String, days: u32) -> CmdResult<Vec<Day>> {
    let first = parse_date(&start)?;
    let days = days.clamp(1, 62);
    let meetings = crate::calendar::meetings(
        &app,
        local_midnight(first),
        local_midnight(first + Days::new(days.into())),
    );
    let series = crate::calendar::series(&app);
    let mut suggester = LazySuggester {
        series: &series,
        model: None,
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    (0..days)
        .map(|i| {
            let date = first + Days::new(i.into());
            let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
            let todays: Vec<Meeting> = meetings
                .iter()
                .filter(|m| m.start < to && m.end > from)
                .cloned()
                .collect();
            let (_, unassigned) = store.classify_meetings(&todays).map_err(err)?;
            let saved = store.timesheet_entries(date, date).map_err(err)?;
            // Gözden geçir'in önerdiği süre (kısa parçalar sayılmaz): bağlantı aynı listeyi açar.
            let unassigned_seconds = store.unassigned(from, to).map_err(err)?.total_seconds;
            let entries = if saved.is_empty() {
                store
                    .propose_timesheet(from, to, &todays)
                    .map_err(err)?
                    .into_iter()
                    .map(|entry| EntryView {
                        id: None,
                        exported: false,
                        entry,
                    })
                    .collect()
            } else {
                saved
                    .iter()
                    .map(|s| EntryView {
                        id: Some(s.id.clone()),
                        exported: s.exported_at.is_some(),
                        entry: s.entry.clone(),
                    })
                    .collect()
            };
            Ok(Day {
                date,
                approved: !saved.is_empty(),
                entries,
                unassigned_seconds,
                meetings: unassigned
                    .into_iter()
                    .map(|meeting| UnassignedMeeting {
                        suggestion: suggester.suggest(&store, &meeting),
                        meeting,
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Gün takvimindeki toplantı: serinin projesi (elle atanan ya da kuraldan) ile.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarMeeting {
    #[serde(flatten)]
    meeting: Meeting,
    project_id: Option<String>,
    /// Seri zaman çizelgesine alınmıyor (yoksayıldı).
    ignored: bool,
    /// Projesi belli olmayan (yoksayılmamış) toplantı için önerilen proje.
    suggestion: Option<MeetingSuggestion>,
}

/// `start` gününden itibaren `days` günün takvim toplantıları (gün takviminde gösterilir).
#[tauri::command]
pub async fn calendar_meetings(
    app: AppHandle,
    start: String,
    days: u32,
) -> CmdResult<Vec<CalendarMeeting>> {
    let first = parse_date(&start)?;
    let days = days.clamp(1, 62);
    let meetings = crate::calendar::meetings(
        &app,
        local_midnight(first),
        local_midnight(first + Days::new(days.into())),
    );
    let series = crate::calendar::series(&app);
    let mut suggester = LazySuggester {
        series: &series,
        model: None,
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let (known, unassigned) = store.classify_meetings(&meetings).map_err(err)?;
    Ok(meetings
        .into_iter()
        .map(|meeting| {
            let project_id = known
                .iter()
                .find(|(m, _)| *m == meeting)
                .map(|(_, p)| p.clone());
            let open = unassigned.contains(&meeting);
            let suggestion = if open {
                suggester.suggest(&store, &meeting)
            } else {
                None
            };
            CalendarMeeting {
                ignored: project_id.is_none() && !open,
                meeting,
                project_id,
                suggestion,
            }
        })
        .collect())
}

/// Günün önerilerini kaydeder (onaylar); onaylı günde "yeniden öner" olarak da kullanılır.
/// Excel'e aktarılmış kayıtlar korunur.
#[tauri::command]
pub async fn approve_timesheet_day(app: AppHandle, date: String) -> CmdResult<()> {
    let date = parse_date(&date)?;
    let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
    let meetings = crate::calendar::meetings(&app, from, to);
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let proposed = store.propose_timesheet(from, to, &meetings).map_err(err)?;
    store.replace_timesheet_day(date, &proposed).map_err(err)
}

/// Takvimdeki toplantı serisini projeye atar (`None`: zaman çizelgesine alma). Atama
/// serinin tüm tekrarlarına uygulanır. `date` günü onaylanmışsa toplantı o güne satır
/// olarak da eklenir (onaylı gün yeniden önerilmeden değişmez).
#[tauri::command]
pub async fn assign_meeting(
    app: AppHandle,
    uid: String,
    project_id: Option<String>,
    date: String,
) -> CmdResult<()> {
    let date = parse_date(&date)?;
    let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
    let meetings: Vec<Meeting> = crate::calendar::meetings(&app, from, to)
        .into_iter()
        .filter(|m| m.uid == uid)
        .collect();
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let approved = !store.timesheet_entries(date, date).map_err(err)?.is_empty();
    // Önceki atamayla güne eklenmiş satırlar: yeniden atamada (A→B ya da yoksay) kaldırılır,
    // yoksa eski satırlar kalıp saatler iki kez yazılırdı.
    let before = if approved {
        store.propose_meetings(from, to, &meetings).map_err(err)?
    } else {
        Vec::new()
    };
    store
        .assign_meeting(&uid, project_id.as_deref())
        .map_err(err)?;
    if approved {
        let after = store.propose_meetings(from, to, &meetings).map_err(err)?;
        store
            .replace_meeting_entries(date, &before, &after)
            .map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn save_timesheet_entry(
    app: AppHandle,
    id: Option<String>,
    entry: TimesheetEntry,
) -> CmdResult<String> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    store
        .save_timesheet_entry(id.as_deref(), &entry)
        .map_err(err)
}

#[tauri::command]
pub async fn delete_timesheet_entry(app: AppHandle, id: String) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    store.delete_timesheet_entry(&id).map_err(err)
}

/// Açıklama önerileri: şablondan gelenler ve son kaydedilenler.
#[tauri::command]
pub async fn timesheet_details(app: AppHandle) -> CmdResult<Vec<String>> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let today = chrono::Local::now().date_naive();
    let mut out: Vec<String> = store
        .timesheet_entries(today - Days::new(90), today)
        .map_err(err)?
        .into_iter()
        .rev()
        .map(|e| e.entry.details)
        .collect();
    out.extend(
        store
            .setting::<Vec<String>>(DETAILS_KEY)
            .map_err(err)?
            .unwrap_or_default(),
    );
    let mut seen = std::collections::HashSet::new();
    out.retain(|d| !d.trim().is_empty() && seen.insert(d.to_lowercase()));
    Ok(out)
}

/// Excel dosyası seçtirir; vazgeçilirse `None`.
#[tauri::command]
pub async fn pick_timesheet_file(app: AppHandle) -> CmdResult<Option<String>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Zaman çizelgesi Excel dosyası")
            .add_filter("Excel", &["xlsx"])
            .blocking_pick_file()
    })
    .await
    .map_err(err)?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    config: TimesheetConfig,
    /// Yeni oluşturulan proje adları.
    created: Vec<String>,
    details: usize,
}

/// Şablonu içe aktarır: firma, danışman ve taraf ayarları; her birim için (yoksa) proje
/// ve eşlemesi; geçmiş açıklamalar. Dosya yolu kayıtların ekleneceği dosya olur.
#[tauri::command]
pub async fn import_timesheet_template(app: AppHandle, path: String) -> CmdResult<Imported> {
    let template = {
        let path = std::path::PathBuf::from(&path);
        tauri::async_runtime::spawn_blocking(move || tracky_xlsx::inspect(&path))
            .await
            .map_err(err)?
            .map_err(err)?
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    config.file_path = Some(path);
    // Excel seçildi: kayıtlar bundan sonra bu dosyaya gider.
    config.sheet_url = None;
    apply_template(&store, config, template)
}

/// Şablon bilgilerini ayarlara işler: firma, danışman ve taraf; her birim için (yoksa)
/// proje ve eşlemesi; geçmiş açıklamalar.
fn apply_template(
    store: &Store,
    mut config: TimesheetConfig,
    template: tracky_xlsx::Template,
) -> CmdResult<Imported> {
    if let Some(c) = template.company {
        config.company = c;
    }
    if let Some(c) = template.consultant {
        config.consultant = c;
    }
    if let Some(p) = template.parties.first() {
        config.default_party = p.clone();
    }
    // Firma müşteri olur; dosyadaki birimlerin (projelerin) müşterisi yoksa ona bağlanır.
    let client_id = match config.company.trim() {
        "" => None,
        company => {
            let clients = store.clients().map_err(err)?;
            let id = match clients
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(company))
            {
                Some(c) => c.id.clone(),
                None => {
                    let c = tracky_core::Client {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: company.to_string(),
                    };
                    store.upsert_client(&c, clients.len() as i64).map_err(err)?;
                    c.id
                }
            };
            Some(id)
        }
    };
    let linked = store.project_clients().map_err(err)?;
    let mut created = Vec::new();
    for division in &template.divisions {
        let tags = store.tags().map_err(err)?;
        let existing = tags
            .iter()
            .find(|t| t.kind == TagKind::Project && t.name.eq_ignore_ascii_case(division));
        let id = match existing {
            Some(t) => t.id.clone(),
            None => {
                let tag = Tag {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: TagKind::Project,
                    name: division.clone(),
                    color: (tags.len() % 8) as u8 + 1,
                };
                store.upsert_tag(&tag, tags.len() as i64).map_err(err)?;
                // Kuralsız proje süre toplamaz; adı başlıkta aranan sözcük olur.
                store
                    .upsert_rule(&Rule {
                        id: uuid::Uuid::new_v4().to_string(),
                        tag_id: tag.id.clone(),
                        field: RuleField::Title,
                        pattern: division.clone(),
                    })
                    .map_err(err)?;
                created.push(division.clone());
                tag.id
            }
        };
        if let Some(client) = &client_id
            && !linked.contains_key(&id)
        {
            store.set_project_client(&id, Some(client)).map_err(err)?;
        }
        if !config.projects.iter().any(|m| m.project_id == id) {
            config.projects.push(ProjectMapping {
                project_id: id,
                division: division.clone(),
                party: None,
                default_details: None,
            });
        }
    }
    store.save_timesheet_config(&config).map_err(err)?;
    let details: Vec<String> = template.details.into_iter().take(MAX_DETAILS).collect();
    store.save_setting(DETAILS_KEY, &details).map_err(err)?;
    Ok(Imported {
        config,
        created,
        details: details.len(),
    })
}

/// Apps Script anahtarını (yoksa üretip) döndürür.
fn sheet_token(store: &Store) -> CmdResult<String> {
    let mut config = store.timesheet_config().map_err(err)?;
    if config.sheet_token.is_empty() {
        config.sheet_token = uuid::Uuid::new_v4().simple().to_string();
        store.save_timesheet_config(&config).map_err(err)?;
    }
    Ok(config.sheet_token)
}

/// Google Sheets tablosuna eklenecek Apps Script (bu kuruluma özgü anahtarla).
#[tauri::command]
pub async fn sheet_script(app: AppHandle) -> CmdResult<String> {
    let token = sheet_token(&lock(&app.state::<Shared>().store))?;
    Ok(tracky_xlsx::sheets::script(&token))
}

/// Google Sheets'e bağlanır: web uygulamasını dener, tablodan şablon bilgilerini içe
/// aktarır; kayıtlar bundan sonra bu tabloya gider.
#[tauri::command]
pub async fn connect_sheet(
    app: AppHandle,
    url: String,
    link: Option<String>,
) -> CmdResult<Imported> {
    let url = tracky_xlsx::sheets::check_url(&url).map_err(err)?;
    let token = sheet_token(&lock(&app.state::<Shared>().store))?;
    let template = {
        let url = url.clone();
        tauri::async_runtime::spawn_blocking(move || tracky_xlsx::sheets::inspect(&url, &token))
            .await
            .map_err(err)?
            .map_err(err)?
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    config.sheet_url = Some(url);
    config.sheet_link = link.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    apply_template(&store, config, template)
}

/// Google Sheets bağlantısını kaldırır; kayıtlar yeniden Excel dosyasına (seçiliyse) gider.
#[tauri::command]
pub async fn disconnect_sheet(app: AppHandle) -> CmdResult<TimesheetConfig> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    config.sheet_url = None;
    store.save_timesheet_config(&config).map_err(err)?;
    Ok(config)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    rows: usize,
    filled: usize,
    inserted: usize,
    /// Daha önce yazıldığı için atlanan (yalnızca Sheets).
    skipped: usize,
    /// Excel yedeği; Sheets'te yok (tablonun sürüm geçmişi var).
    backup: Option<String>,
    /// Excel dosyası ya da Sheets sayfasının adı.
    target: String,
    sheets: bool,
}

/// `start`'tan itibaren `days` gündeki onaylı ve aktarılmamış kayıtları Excel dosyasına
/// ya da (bağlıysa) Google Sheets tablosuna ekler.
#[tauri::command]
pub async fn export_timesheet(app: AppHandle, start: String, days: u32) -> CmdResult<Exported> {
    if EXPORTING.swap(true, Ordering::Acquire) {
        return Err("Excel'e aktarım zaten sürüyor.".into());
    }
    let _guard = ExportGuard;
    let first = parse_date(&start)?;
    let last = first + Days::new(u64::from(days.clamp(1, 62)) - 1);
    let (config, pending) = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let config = store.timesheet_config().map_err(err)?;
        let pending: Vec<_> = store
            .timesheet_entries(first, last)
            .map_err(err)?
            .into_iter()
            .filter(|e| e.exported_at.is_none())
            .collect();
        (config, pending)
    };
    if config.sheet_url.is_none() && config.file_path.is_none() {
        return Err(NO_TARGET.into());
    }
    if pending.is_empty() {
        return Err("Aktarılacak onaylı kayıt yok; önce günleri onayla.".into());
    }
    // Firmaya açıklamasız satır gitmesin.
    let blank: Vec<String> = pending
        .iter()
        .filter(|e| e.entry.details.trim().is_empty())
        .map(|e| {
            format!(
                "{} {}",
                e.entry.date.format("%d.%m"),
                e.entry.start.format("%H:%M")
            )
        })
        .collect();
    if !blank.is_empty() {
        return Err(format!(
            "Açıklaması boş {} satır var ({}); doldurup tekrar dene.",
            blank.len(),
            blank.join(", ")
        ));
    }
    let rows: Vec<tracky_xlsx::Row> = pending
        .iter()
        .map(|e| tracky_xlsx::Row {
            date: e.entry.date,
            start: e.entry.start,
            hours: e.entry.hours,
            kind: e.entry.kind.label().to_string(),
            details: e.entry.details.clone(),
            party: e.entry.party.clone(),
            division: e.entry.division.clone(),
        })
        .collect();
    let ids: Vec<String> = pending.iter().map(|e| e.id.clone()).collect();
    let consultant = config.consultant.clone();
    let (exported, target) = match (config.sheet_url.clone(), config.file_path.clone()) {
        (Some(url), _) => {
            let token = config.sheet_token.clone();
            let keyed: Vec<_> = ids.iter().cloned().zip(rows).collect();
            let target = ExportTarget::Sheets {
                url: url.clone(),
                token: token.clone(),
            };
            let done = tauri::async_runtime::spawn_blocking(move || {
                tracky_xlsx::sheets::append(&url, &token, &consultant, &keyed)
            })
            .await
            .map_err(err)?
            .map_err(err)?;
            let exported = Exported {
                rows: ids.len(),
                filled: done.filled,
                inserted: done.inserted,
                skipped: done.skipped,
                backup: None,
                target: done.sheet,
                sheets: true,
            };
            (exported, target)
        }
        (None, Some(path)) => {
            let file = std::path::PathBuf::from(&path);
            let written = file.clone();
            let done = tauri::async_runtime::spawn_blocking(move || {
                tracky_xlsx::append(&written, &consultant, &rows)
            })
            .await
            .map_err(err)?
            .map_err(err)?;
            let exported = Exported {
                rows: ids.len(),
                filled: done.filled,
                inserted: done.inserted,
                skipped: 0,
                backup: Some(done.backup.display().to_string()),
                target: path,
                sheets: false,
            };
            let target = ExportTarget::Excel {
                modified: modified(&file),
                path: file,
                backup: done.backup,
            };
            (exported, target)
        }
        (None, None) => return Err(NO_TARGET.into()),
    };
    lock(&app.state::<Shared>().store)
        .mark_timesheet_exported(&ids, Utc::now())
        .map_err(err)?;
    *lock(&LAST_EXPORT) = Some(LastExport { ids, target });
    Ok(exported)
}

/// Son aktarımı geri alır: Sheets'te yazılan satırlar silinir ya da boşaltılır, Excel dosyası
/// aktarım öncesi yedeğinden geri yüklenir. Kayıtlar yeniden aktarılmamış sayılır.
#[tauri::command]
pub async fn undo_last_export(app: AppHandle) -> CmdResult<String> {
    if EXPORTING.swap(true, Ordering::Acquire) {
        return Err("Aktarım sürüyor; bitince tekrar dene.".into());
    }
    let _guard = ExportGuard;
    let Some(last) = lock(&LAST_EXPORT).take() else {
        return Err("Geri alınacak aktarım yok.".into());
    };
    let message = match &last.target {
        ExportTarget::Sheets { url, token } => {
            let (url, token, ids) = (url.clone(), token.clone(), last.ids.clone());
            let done = tauri::async_runtime::spawn_blocking(move || {
                tracky_xlsx::sheets::undo(&url, &token, &ids)
            })
            .await
            .map_err(err)?;
            let done = match done {
                Ok(d) => d,
                Err(e) => {
                    *lock(&LAST_EXPORT) = Some(last);
                    return Err(err(e));
                }
            };
            let missing = if done.missing > 0 {
                format!(" {} satır tabloda bulunamadı.", done.missing)
            } else {
                String::new()
            };
            format!(
                "Aktarım geri alındı: {} satır silindi, {} satır boşaltıldı.{missing}",
                done.removed, done.cleared
            )
        }
        ExportTarget::Excel {
            path,
            backup,
            modified: at,
        } => {
            if modified(path) != *at {
                let msg = format!(
                    "Excel dosyası aktarımdan sonra değişmiş; üzerine yazılmadı. Aktarım öncesi yedek: {}",
                    backup.display()
                );
                return Err(msg);
            }
            if let Err(e) = std::fs::copy(backup, path) {
                let msg = format!(
                    "Excel dosyası geri yüklenemedi (Excel'de açıksa kapatıp tekrar dene): {e}"
                );
                *lock(&LAST_EXPORT) = Some(last);
                return Err(msg);
            }
            "Aktarım geri alındı: Excel dosyası aktarım öncesi haline döndü.".to_string()
        }
    };
    lock(&app.state::<Shared>().store)
        .unmark_timesheet_exported(&last.ids)
        .map_err(err)?;
    Ok(message)
}
