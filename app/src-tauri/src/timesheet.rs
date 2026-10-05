//! Zaman çizelgesi komutları: firmaların zaman çizelgeleri (her biri yalnızca kendisine bağlı
//! projelerin işini alır), günün satırları (kaydedilmiş satırlar ve takipten gelen canlı
//! öneriler), satırları düzenleme, gizleme ve birleştirme, şablonu içe aktarma ve satırları
//! çizelgenin Excel dosyasına ya da Google Sheets tablosuna ekleme.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::meeting_suggest::{MeetingSuggester, MeetingSuggestion};
use tracky_core::store::{DayRow, TimesheetContext};
use tracky_core::timesheet::{
    self, Meeting, Piece, ProjectMapping, Timesheet, TimesheetConfig, TimesheetEntry,
};
use tracky_core::{Rule, RuleField, Store, Tag, TagKind};

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Aktarım sürüyor: ikinci bir aktarım (çift tıklama) aynı kayıtları dosyaya ikinci kez
/// yazmasın diye reddedilir.
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
/// yazılmamışken işaretlenirdi. Depo kilidi tutulurken çağrılmalı (aktarım gönderdiği kayıtları
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
const NO_TARGET: &str = "Önce bu zaman çizelgesinin Excel dosyasını seç ya da Google Sheets'e bağla (Ayarlar → Zaman çizelgeleri)";
const NO_SHEET: &str = "Zaman çizelgesi bulunamadı; Ayarlar → Zaman çizelgeleri'nden kontrol et.";

/// Bir günün bir zaman çizelgesindeki satırları.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    date: NaiveDate,
    /// Kaydedilmiş satırlar ve canlı öneriler, başlangıca göre.
    entries: Vec<DayRow>,
    /// Gizlenen satır sayısı.
    hidden: usize,
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

/// Arayüzden gelen satır: kaydedilmişse kimliğiyle, canlı öneriyse yalnızca içeriğiyle.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowRef {
    id: Option<String>,
    entry: TimesheetEntry,
}

/// Birleştirmede silinen (geri almada aynen geri gelen) satır.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    id: String,
    entry: TimesheetEntry,
}

fn parse_date(s: &str) -> CmdResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("geçersiz tarih {s}: {e}"))
}

/// Günün sınırları ve o güne düşen toplantılar.
fn day_meetings(
    meetings: &[Meeting],
    date: NaiveDate,
) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>, Vec<Meeting>) {
    let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
    let todays = meetings
        .iter()
        .filter(|m| m.start < to && m.end > from)
        .cloned()
        .collect();
    (from, to, todays)
}

/// Günlerin parçaları ([`Store::timesheet_pieces`]), her gün için bir kez.
struct DayPieces<'a> {
    store: &'a Store,
    ctx: &'a TimesheetContext,
    meetings: &'a [Meeting],
    days: HashMap<NaiveDate, Vec<Piece>>,
}

impl DayPieces<'_> {
    fn get(&mut self, date: NaiveDate) -> CmdResult<&[Piece]> {
        if !self.days.contains_key(&date) {
            let (from, to, todays) = day_meetings(self.meetings, date);
            let pieces = self
                .store
                .timesheet_pieces(self.ctx, from, to, &todays)
                .map_err(err)?;
            self.days.insert(date, pieces);
        }
        Ok(&self.days[&date])
    }
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

/// `from`–`to` (dahil) arasında zaman çizelgelerine henüz aktarılmamış işi olan günler: bir
/// çizelgesinde aktarılmamış satırı (kaydedilmiş ya da canlı) olan günler. Yazılacak yeri ve
/// projesi olan çizelge yoksa boştur.
pub fn unexported_days(app: &AppHandle, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let meetings =
        crate::calendar::meetings(app, local_midnight(from), local_midnight(to + Days::new(1)));
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let Ok(ctx) = store.timesheet_context() else {
        return Vec::new();
    };
    let sheets: Vec<&Timesheet> = ctx
        .config
        .timesheets
        .iter()
        .filter(|t| t.has_target() && !t.projects.is_empty())
        .collect();
    if sheets.is_empty() {
        return Vec::new();
    }
    let pending = |date: NaiveDate| -> Result<bool, tracky_core::StoreError> {
        let (start, end, todays) = day_meetings(&meetings, date);
        let pieces = store.timesheet_pieces(&ctx, start, end, &todays)?;
        for sheet in &sheets {
            let day = store.timesheet_day(&ctx, sheet, date, &pieces)?;
            if day.rows.iter().any(|r| !r.exported) {
                return Ok(true);
            }
        }
        Ok(false)
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

/// `timesheet_id` çizelgesinde `start` gününden itibaren `days` günün satırları.
#[tauri::command]
pub async fn timesheet_days(
    app: AppHandle,
    timesheet_id: String,
    start: String,
    days: u32,
) -> CmdResult<Vec<Day>> {
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
    let ctx = store.timesheet_context().map_err(err)?;
    let sheet = ctx.config.timesheet(&timesheet_id).ok_or(NO_SHEET)?;
    (0..days)
        .map(|i| {
            let date = first + Days::new(i.into());
            let (from, to, todays) = day_meetings(&meetings, date);
            let (_, unassigned) = store.classify_meetings(&todays).map_err(err)?;
            // Gözden geçir'in önerdiği süre (kısa parçalar sayılmaz): bağlantı aynı listeyi açar.
            let unassigned_seconds = store.unassigned(from, to).map_err(err)?.total_seconds;
            let pieces = store
                .timesheet_pieces(&ctx, from, to, &todays)
                .map_err(err)?;
            let day = store
                .timesheet_day(&ctx, sheet, date, &pieces)
                .map_err(err)?;
            Ok(Day {
                date,
                entries: day.rows,
                hidden: day.hidden,
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

/// Takvimdeki toplantı serisini projeye atar (`None`: zaman çizelgesine alma). Atama serinin
/// tüm tekrarlarına uygulanır; toplantının satırı canlı öneri olarak gelir. Kaydedilmiş bir
/// toplantı satırının projesi değişirse satır "takipte değişti" olur.
#[tauri::command]
pub async fn assign_meeting(
    app: AppHandle,
    uid: String,
    project_id: Option<String>,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .assign_meeting(&uid, project_id.as_deref())
        .map_err(err)
}

/// Satırı kaydeder: kaydedilmiş satırı günceller, canlı öneriyi ya da elle eklenen satırı ekler.
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

/// Satırı gizler (siler); canlı öneri gizlenmiş olarak kaydedilir. Satırın kimliği döner.
#[tauri::command]
pub async fn dismiss_timesheet_entry(
    app: AppHandle,
    id: Option<String>,
    entry: TimesheetEntry,
) -> CmdResult<String> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    store
        .dismiss_timesheet_entry(id.as_deref(), &entry)
        .map_err(err)
}

/// Gizlenen satırları geri getirir (gizlemenin geri alınması).
#[tauri::command]
pub async fn undismiss_timesheet_entries(app: AppHandle, ids: Vec<String>) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    store.undismiss_timesheet_entries(&ids).map_err(err)
}

/// Günün `timesheet_id` çizelgesinde gizlenen satırlarını geri getirir; kimliklerini döndürür.
#[tauri::command]
pub async fn restore_hidden_entries(
    app: AppHandle,
    timesheet_id: String,
    date: String,
) -> CmdResult<Vec<String>> {
    let date = parse_date(&date)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let config = store.timesheet_config().map_err(err)?;
    let sheet = config.timesheet(&timesheet_id).ok_or(NO_SHEET)?;
    store.restore_hidden(sheet, date).map_err(err)
}

/// Satırı tamamen siler (elle eklenen satır; gizlenen canlı önerinin geri alınması).
#[tauri::command]
pub async fn delete_timesheet_entry(app: AppHandle, id: String) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    store.delete_timesheet_entry(&id).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Merged {
    id: String,
    /// Geri almak için: silinen kaydedilmiş satırlar.
    removed: Vec<Removed>,
}

/// Aynı günün ve projenin satırlarını tek satırda birleştirir.
#[tauri::command]
pub async fn merge_timesheet_entries(app: AppHandle, rows: Vec<RowRef>) -> CmdResult<Merged> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let rows: Vec<(Option<String>, TimesheetEntry)> =
        rows.into_iter().map(|r| (r.id, r.entry)).collect();
    let (id, removed) = store.merge_timesheet_entries(&rows).map_err(err)?;
    Ok(Merged {
        id,
        removed: removed
            .into_iter()
            .map(|(id, entry)| Removed { id, entry })
            .collect(),
    })
}

/// Birleştirmeyi geri alır.
#[tauri::command]
pub async fn unmerge_timesheet_entries(
    app: AppHandle,
    id: String,
    removed: Vec<Removed>,
) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let removed: Vec<(String, TimesheetEntry)> =
        removed.into_iter().map(|r| (r.id, r.entry)).collect();
    store.unmerge_timesheet_entries(&id, &removed).map_err(err)
}

/// Takipte değişen satırları günceller: süreleri projede kalan işe iner, hiç iş kalmayan
/// satır silinir. Silinen satır sayısı.
#[tauri::command]
pub async fn refresh_timesheet_entries(app: AppHandle, ids: Vec<String>) -> CmdResult<usize> {
    let dates: Vec<NaiveDate> = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let mut dates = Vec::new();
        for id in &ids {
            if let Some(s) = store.timesheet_entry(id).map_err(err)? {
                dates.push(s.entry.date);
            }
        }
        dates
    };
    let (Some(first), Some(last)) = (dates.iter().min(), dates.iter().max()) else {
        return Ok(0);
    };
    let meetings = crate::calendar::meetings(
        &app,
        local_midnight(*first),
        local_midnight(*last + Days::new(1)),
    );
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let ctx = store.timesheet_context().map_err(err)?;
    let mut pieces = DayPieces {
        store: &store,
        ctx: &ctx,
        meetings: &meetings,
        days: HashMap::new(),
    };
    let mut removed = 0;
    for id in &ids {
        let Some(saved) = store.timesheet_entry(id).map_err(err)? else {
            continue;
        };
        let day = pieces.get(saved.entry.date)?;
        if !store.refresh_timesheet_entry(id, day).map_err(err)? {
            removed += 1;
        }
    }
    Ok(removed)
}

/// "Yeniden öner": günün bu çizelgedeki aktarılmamış satırlarını (düzenlemeler, elle eklenenler,
/// gizlenenler) siler; iş yeniden canlı öneri olur.
#[tauri::command]
pub async fn reset_timesheet_day(
    app: AppHandle,
    timesheet_id: String,
    date: String,
) -> CmdResult<usize> {
    let date = parse_date(&date)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let config = store.timesheet_config().map_err(err)?;
    let sheet = config.timesheet(&timesheet_id).ok_or(NO_SHEET)?;
    store.reset_timesheet_day(sheet, date).map_err(err)
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
    /// İçe aktarılan (yeni ya da güncellenen) çizelge.
    timesheet_id: String,
    /// Yeni oluşturulan proje adları.
    created: Vec<String>,
    details: usize,
}

/// İçe aktarılan dosyanın çizelgesi: `id` verilmişse o, yoksa aynı dosyaya ya da tabloya bağlı
/// çizelge (`same`; aynı dosyayı yeniden seçmek ikinci çizelge açmasın), o da yoksa sona eklenen
/// yeni çizelge. (sıra, yeni mi)
fn sheet_slot(
    config: &mut TimesheetConfig,
    id: Option<&str>,
    same: impl Fn(&Timesheet) -> bool,
) -> (usize, bool) {
    let found = match id {
        Some(id) => config.timesheets.iter().position(|t| t.id == id),
        None => config.timesheets.iter().position(same),
    };
    match found {
        Some(i) => (i, false),
        None => {
            config.timesheets.push(Timesheet {
                id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            });
            (config.timesheets.len() - 1, true)
        }
    }
}

/// Şablonu içe aktarır ve dosyayı çizelgenin (`timesheet_id` yoksa yeni çizelgenin) Excel
/// dosyası yapar: firma, danışman, taraf ve birimler dosyadan alınır; yeni çizelgede birimler
/// proje olur ([`apply_template`]).
#[tauri::command]
pub async fn import_timesheet_template(
    app: AppHandle,
    timesheet_id: Option<String>,
    path: String,
) -> CmdResult<Imported> {
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
    let (i, new) = sheet_slot(&mut config, timesheet_id.as_deref(), |t| {
        t.file_path.as_deref() == Some(path.as_str())
    });
    config.timesheets[i].file_path = Some(path);
    // Excel seçildi: kayıtlar bundan sonra bu dosyaya gider.
    config.timesheets[i].sheet_url = None;
    apply_template(&store, config, i, template, new)
}

/// Şablon bilgilerini `i` çizelgesine işler: firma, danışman, taraf ve birimler. Yeni çizelgede
/// her birim (yoksa) proje olur ve başka çizelgeye bağlı değilse bu çizelgeye bağlanır; firma
/// müşteri olur ve müşterisi olmayan bu projeler ona bağlanır. Var olan çizelgeye yeniden içe
/// aktarmada proje oluşturulmaz (kullanıcının sildiği birim projeleri geri gelmesin).
fn apply_template(
    store: &Store,
    mut config: TimesheetConfig,
    i: usize,
    template: tracky_xlsx::Template,
    new: bool,
) -> CmdResult<Imported> {
    let sheet = &mut config.timesheets[i];
    if let Some(c) = template.company {
        sheet.company = c;
    }
    if let Some(c) = template.consultant {
        sheet.consultant = c;
    }
    if let Some(p) = template.parties.first() {
        sheet.default_party = p.clone();
    }
    sheet.divisions.extend(template.divisions.iter().cloned());
    let company = sheet.company.trim().to_string();
    let mut created = Vec::new();
    if new {
        // Firma müşteri olur; dosyadaki birimlerin (projelerin) müşterisi yoksa ona bağlanır.
        let client_id = match company.as_str() {
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
            if config.timesheet_of(&id).is_none() {
                config.timesheets[i].projects.push(ProjectMapping {
                    project_id: id,
                    division: division.clone(),
                    party: None,
                    default_details: None,
                });
            }
        }
    }
    let timesheet_id = config.timesheets[i].id.clone();
    store.save_timesheet_config(&config).map_err(err)?;
    // Açıklama önerileri: dosyanınkiler önce, önceki çizelgelerinkiler arkada.
    let mut details: Vec<String> = template.details;
    details.extend(
        store
            .setting::<Vec<String>>(DETAILS_KEY)
            .map_err(err)?
            .unwrap_or_default(),
    );
    let mut seen = std::collections::HashSet::new();
    details.retain(|d| seen.insert(d.to_lowercase()));
    details.truncate(MAX_DETAILS);
    store.save_setting(DETAILS_KEY, &details).map_err(err)?;
    Ok(Imported {
        config: store.timesheet_config().map_err(err)?,
        timesheet_id,
        created,
        details: details.len(),
    })
}

/// Apps Script anahtarını (yoksa üretip) döndürür; bütün tablolarda aynıdır.
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

/// Çizelgeyi (`timesheet_id` yoksa yeni çizelgeyi) Google Sheets'e bağlar: web uygulamasını
/// dener, tablodan şablon bilgilerini içe aktarır; çizelgenin kayıtları bundan sonra bu tabloya
/// gider.
#[tauri::command]
pub async fn connect_sheet(
    app: AppHandle,
    timesheet_id: Option<String>,
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
    let (i, new) = sheet_slot(&mut config, timesheet_id.as_deref(), |t| {
        t.sheet_url.as_deref() == Some(url.as_str())
    });
    config.timesheets[i].sheet_url = Some(url);
    config.timesheets[i].sheet_link = link.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    apply_template(&store, config, i, template, new)
}

/// Çizelgenin Google Sheets bağlantısını kaldırır; kayıtları yeniden Excel dosyasına (seçiliyse)
/// gider.
#[tauri::command]
pub async fn disconnect_sheet(app: AppHandle, timesheet_id: String) -> CmdResult<TimesheetConfig> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    let sheet = config
        .timesheets
        .iter_mut()
        .find(|t| t.id == timesheet_id)
        .ok_or(NO_SHEET)?;
    sheet.sheet_url = None;
    store.save_timesheet_config(&config).map_err(err)?;
    Ok(config)
}

/// Çizelgeyi kaldırır; projeleri hiçbir çizelgeye gitmez. Kaydedilmiş satırlar silinmez.
#[tauri::command]
pub async fn remove_timesheet(app: AppHandle, timesheet_id: String) -> CmdResult<TimesheetConfig> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let mut config = store.timesheet_config().map_err(err)?;
    config.timesheets.retain(|t| t.id != timesheet_id);
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

/// `rows` satırlarını `timesheet_id` çizelgesinin Excel dosyasına ya da (bağlıysa) Google Sheets
/// tablosuna ekler; canlı öneriler önce kaydedilir. Satırlardan biri çizelgenin projesine ait
/// değilse, açıklaması boşsa ya da takipte değiştiyse (işi raporda başka projeye alınmış) hiçbiri
/// gönderilmez.
#[tauri::command]
pub async fn export_timesheet(
    app: AppHandle,
    timesheet_id: String,
    rows: Vec<RowRef>,
) -> CmdResult<Exported> {
    if EXPORTING.swap(true, Ordering::Acquire) {
        return Err("Aktarım zaten sürüyor.".into());
    }
    let _guard = ExportGuard;
    let (Some(first), Some(last)) = (
        rows.iter().map(|r| r.entry.date).min(),
        rows.iter().map(|r| r.entry.date).max(),
    ) else {
        return Err("Gönderilecek satır yok.".into());
    };
    let meetings = crate::calendar::meetings(
        &app,
        local_midnight(first),
        local_midnight(last + Days::new(1)),
    );
    let (sheet, token, ids, pending) = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let ctx = store.timesheet_context().map_err(err)?;
        let sheet = ctx
            .config
            .timesheet(&timesheet_id)
            .cloned()
            .ok_or(NO_SHEET)?;
        if !sheet.has_target() {
            return Err(NO_TARGET.into());
        }
        // Kaydedilmiş satırlar veritabanından okunur (arayüzdeki kopya eski olabilir).
        let mut entries: Vec<(Option<String>, TimesheetEntry)> = Vec::new();
        for r in rows {
            let Some(id) = r.id else {
                entries.push((None, r.entry));
                continue;
            };
            let saved = store
                .timesheet_entry(&id)
                .map_err(err)?
                .filter(|s| !s.dismissed && s.exported_at.is_none())
                .ok_or(
                    "Satırlar değişti (silinmiş ya da aktarılmış); sayfa yenilendi, tekrar dene.",
                )?;
            entries.push((Some(id), saved.entry));
        }
        let label =
            |e: &TimesheetEntry| format!("{} {}", e.date.format("%d.%m"), e.start.format("%H:%M"));
        // Yalnızca bu çizelgenin projeleri: başka projenin işi bu firmaya gitmesin.
        if let Some((_, e)) = entries.iter().find(|(_, e)| !sheet.includes(&e.project_id)) {
            let name = ctx.project_name(&e.project_id).unwrap_or(&e.division);
            return Err(format!(
                "{} satırının projesi ({name}) {} zaman çizelgesine bağlı değil; gönderilmedi.",
                label(e),
                sheet.company
            ));
        }
        // Firmaya açıklamasız satır gitmesin.
        let blank: Vec<String> = entries
            .iter()
            .filter(|(_, e)| e.details.trim().is_empty())
            .map(|(_, e)| label(e))
            .collect();
        if !blank.is_empty() {
            return Err(format!(
                "Açıklaması boş {} satır var ({}); doldurup tekrar dene.",
                blank.len(),
                blank.join(", ")
            ));
        }
        // İşi raporda başka projeye alınmış satır, bu firmanın işi değildir artık.
        let mut pieces = DayPieces {
            store: &store,
            ctx: &ctx,
            meetings: &meetings,
            days: HashMap::new(),
        };
        let mut stale = Vec::new();
        for (_, e) in &entries {
            if timesheet::stale_hours(pieces.get(e.date)?, e).is_some() {
                stale.push(label(e));
            }
        }
        if !stale.is_empty() {
            return Err(format!(
                "Takipte değişen {} satır var ({}): işi raporda başka projeye alınmış. Satırı güncelle ya da sil.",
                stale.len(),
                stale.join(", ")
            ));
        }
        // Canlı öneriler kaydedilir (hep ya da hiç); aynı satır iki kez gönderilmez.
        let ids: Vec<String> = store
            .atomic(|s| {
                entries
                    .iter()
                    .map(|(id, e)| match id {
                        Some(id) => Ok(id.clone()),
                        None => s.save_timesheet_entry(None, e),
                    })
                    .collect()
            })
            .map_err(err)?;
        let mut seen = std::collections::HashSet::new();
        let pending: Vec<(String, TimesheetEntry)> = ids
            .into_iter()
            .zip(entries.into_iter().map(|(_, e)| e))
            .filter(|(id, _)| seen.insert(id.clone()))
            .collect();
        let ids: Vec<String> = pending.iter().map(|(id, _)| id.clone()).collect();
        (sheet, ctx.config.sheet_token.clone(), ids, pending)
    };
    let rows: Vec<tracky_xlsx::Row> = pending
        .iter()
        .map(|(_, e)| tracky_xlsx::Row {
            date: e.date,
            start: e.start,
            hours: e.hours,
            kind: e.kind.label().to_string(),
            details: e.details.clone(),
            party: e.party.clone(),
            division: e.division.clone(),
        })
        .collect();
    let consultant = sheet.consultant.clone();
    let (exported, target) = match (sheet.sheet_url.clone(), sheet.file_path.clone()) {
        (Some(url), _) => {
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
        .mark_timesheet_exported(&ids, Utc::now(), &sheet.id)
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
