//! Zaman çizelgesi komutları: firmaların zaman çizelgeleri (her biri yalnızca kendisine bağlı
//! projelerin işini alır), günün satırları (kaydedilmiş satırlar ve takipten gelen canlı
//! öneriler), satırları düzenleme, gizleme ve birleştirme, şablonu içe aktarma ve satırları
//! çizelgenin Excel dosyasına ya da Google Sheets tablosuna ekleme.
//!
//! Dosyadaki satırlar da okunur: Kum'un aktardığı satırlar dosyadaki satırlarıyla eşlenir,
//! Kum dışında girilen satırlar ayrıca gösterilir ve buradan değiştirilir. Aktarılmış satır
//! düzenlenince, silinince ya da takipte değişip güncellenince dosyadaki satırı da değişir.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::attendance::Attendance;
use tracky_core::meeting_suggest::{MeetingSuggester, MeetingSuggestion};
use tracky_core::store::{DayRow, SavedEntry, TimesheetContext};
use tracky_core::timesheet::{
    self, FileRow, Meeting, Piece, ProjectMapping, Timesheet, TimesheetConfig, TimesheetEntry,
};
use tracky_core::{Rule, RuleField, Store, Tag, TagKind};

use crate::google::GoogleAuth;
use crate::lock;
use crate::tracking::{Shared, local_midnight};

pub(crate) mod export;
pub(crate) mod setup;
mod target;
#[cfg(test)]
mod tests;

use export::not_exporting;
use target::*;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
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
            self.days.insert(date, pieces.pieces);
        }
        Ok(&self.days[&date])
    }
}

/// Kapatılan günler (YYYY-MM-DD).
#[tauri::command]
pub async fn closed_days(app: AppHandle) -> CmdResult<Vec<String>> {
    lock(&app.state::<Shared>().store)
        .closed_days()
        .map_err(err)
}

/// Günü kapatıldı işaretler ya da yeniden açar.
#[tauri::command]
pub async fn set_day_closed(app: AppHandle, date: String, closed: bool) -> CmdResult<()> {
    parse_date(&date)?;
    lock(&app.state::<Shared>().store)
        .set_day_closed(&date, closed)
        .map_err(err)
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
    /// Katılım ve görüşmeye göre süre ([`tracky_core::attendance`]).
    attendance: Option<Attendance>,
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
    let config = store.timesheet_config().map_err(err)?;
    let mut attendance = store
        .meeting_attendance(&config, &meetings)
        .map_err(err)?
        .into_iter();
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
                attendance: attendance.next(),
            }
        })
        .collect())
}

/// Toplantının bu tekrarına katılıp katılmadığını kaydeder (`key`:
/// [`tracky_core::attendance::key`]; `None`: cevabı geri al, Kum karar versin).
#[tauri::command]
pub async fn answer_meeting(app: AppHandle, key: String, attended: Option<bool>) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .answer_meeting(&key, attended)
        .map_err(err)
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

/// Aktarılmış satır ve aktarıldığı çizelgenin dosyası; satır aktarılmamışsa `None` (depo kilidi
/// tutulurken).
fn exported_entry(
    store: &Store,
    id: Option<&str>,
) -> CmdResult<Option<(tracky_core::store::SavedEntry, Timesheet, FileTarget)>> {
    let Some(saved) = id
        .map(|id| store.timesheet_entry(id))
        .transpose()
        .map_err(err)?
        .flatten()
        .filter(|s| s.exported_at.is_some() && !s.dismissed)
    else {
        return Ok(None);
    };
    let sheet_id = saved.timesheet_id.clone().unwrap_or_default();
    let (sheet, target) = FileTarget::load(store, &sheet_id).map_err(|e| {
        if e == NO_SHEET {
            "Satırın aktarıldığı zaman çizelgesi artık yok; değiştirilemez.".to_string()
        } else {
            e
        }
    })?;
    Ok(Some((saved, sheet, target)))
}

/// Satırı kaydeder: kaydedilmiş satırı günceller, canlı öneriyi ya da elle eklenen satırı ekler.
/// Aktarılmış satırda değişiklik önce dosyadaki satırına yazılır (`sheet_row`: dosyadaki satır
/// numarası, bilinmiyorsa satır içeriğinden bulunur), sonra Kum'a.
#[tauri::command]
pub async fn save_timesheet_entry(
    app: AppHandle,
    id: Option<String>,
    entry: TimesheetEntry,
    sheet_row: Option<u32>,
) -> CmdResult<String> {
    {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        not_exporting()?;
        if exported_entry(&store, id.as_deref())?.is_none() {
            return store
                .save_timesheet_entry(id.as_deref(), &entry)
                .map_err(err);
        }
    };
    // Satır sıra gelince okunur: önceki düzenleme (sırada bekleyen) dosyaya ve Kum'a yazılmış olsun.
    let _write = FILE_WRITES.lock().await;
    let (saved, sheet, target) =
        exported_entry(&lock(&app.state::<Shared>().store), id.as_deref())?
            .ok_or("Satır değişti; sayfa yenilendi, tekrar dene.")?;
    // Yalnızca satırın alanları değişir; proje, tarih, gerçek süre ve aralıklar Kum'da kalır.
    let entry = TimesheetEntry {
        date: saved.entry.date,
        project_id: saved.entry.project_id.clone(),
        actual_hours: saved.entry.actual_hours,
        coverage: saved.entry.coverage.clone(),
        ..entry
    };
    if !(entry.hours > 0.0 && entry.hours <= 24.0) {
        return Err("saat 0 ile 24 arasında olmalı".into());
    }
    if entry.details.trim().is_empty() {
        return Err("Açıklama boş olamaz: satır firmanın dosyasında.".into());
    }
    let expect = kum_row(&saved, sheet_row.unwrap_or(0), &sheet);
    target.update(&sheet.consultant, &expect, &entry).await?;
    lock(&app.state::<Shared>().store)
        .save_exported_entry(&saved.id, &entry, false, Some(&sheet.consultant))
        .map_err(err)?;
    Ok(saved.id)
}

/// Satırı gizler (siler); canlı öneri gizlenmiş olarak kaydedilir. Satırın kimliği döner.
/// Aktarılmış satır dosyadan da kaldırılır (geri alınmaz; "geri getir" ile yeniden gönderilir).
#[tauri::command]
pub async fn dismiss_timesheet_entry(
    app: AppHandle,
    id: Option<String>,
    entry: TimesheetEntry,
    sheet_row: Option<u32>,
) -> CmdResult<String> {
    {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        not_exporting()?;
        if exported_entry(&store, id.as_deref())?.is_none() {
            return store
                .dismiss_timesheet_entry(id.as_deref(), &entry)
                .map_err(err);
        }
    };
    let _write = FILE_WRITES.lock().await;
    let (saved, sheet, target) =
        exported_entry(&lock(&app.state::<Shared>().store), id.as_deref())?
            .ok_or("Satır değişti; sayfa yenilendi, tekrar dene.")?;
    let expect = kum_row(&saved, sheet_row.unwrap_or(0), &sheet);
    if let Err(e) = target.remove(&expect, Some(&saved.id)).await {
        // Satır dosyadan elle silinmişse kaldırılacak bir şey yok: Kum'da da çekilir (yoksa
        // silme hep "satır değişmiş" hatası verirdi).
        if e != tracky_xlsx::Error::Changed.to_string() || !target.lost(&saved.entry).await? {
            return Err(e);
        }
    }
    lock(&app.state::<Shared>().store)
        .withdraw_exported_entry(&saved.id, true)
        .map_err(err)?;
    Ok(saved.id)
}

/// Dosyadaki bir satır ve (Kum aktardıysa) Kum'daki kaydı.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRowView {
    #[serde(flatten)]
    row: FileRow,
    /// Satırı Kum aktardıysa kaydın kimliği; Kum dışında girilen satırda `None`.
    entry_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRows {
    /// Dönemin dosyadaki satırları (başka danışmanlarınki hariç), dosyadaki sırayla.
    rows: Vec<SheetRowView>,
    /// Bu çizelgeye aktarılmış olup dosyada bulunamayan Kum kayıtları (dosyada silinmiş ya da
    /// başlangıcı değiştirilmiş).
    missing: Vec<String>,
    /// Dosyada değiştirildiği için Kum'da da güncellenen kayıt sayısı (satırlar yeniden okunmalı).
    synced: usize,
}

/// `timesheet_id` çizelgesinin dosyasındaki `start` gününden itibaren `days` günün satırları.
/// Kum'un aktardığı satırlar kayıtlarıyla eşlenir ([`timesheet::link_file_rows`]); dosyada
/// değiştirilmiş satırın değerleri Kum'daki kayda da geçer (firmaya giden dosyadakidir).
#[tauri::command]
pub async fn sheet_rows(
    app: AppHandle,
    timesheet_id: String,
    start: String,
    days: u32,
) -> CmdResult<SheetRows> {
    let first = parse_date(&start)?;
    let last = first + Days::new(u64::from(days.clamp(1, 62)) - 1);
    let (sheet, target) = FileTarget::load(&lock(&app.state::<Shared>().store), &timesheet_id)?;
    let mut rows = target.list(first, last).await?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let exported: Vec<tracky_core::store::SavedEntry> = store
        .timesheet_entries(first, last)
        .map_err(err)?
        .into_iter()
        .filter(|s| s.exported_at.is_some() && s.timesheet_id.as_deref() == Some(sheet.id.as_str()))
        .collect();
    // Ortak tabloda başka danışmanların satırları gösterilmez; Kum'un eski danışman adıyla
    // yazdığı satırlar (ad sonradan değişti) bizimdir.
    rows.retain(|r| {
        mine(&sheet.consultant, r)
            || exported
                .iter()
                .filter_map(|s| s.consultant.as_deref())
                .any(|c| mine(c, r))
    });
    let entries: Vec<&TimesheetEntry> = exported.iter().map(|s| &s.entry).collect();
    let links = timesheet::link_file_rows(&rows, &entries);
    // Aktarım sürerken Kum'daki kayıtlara dokunulmaz (bir sonraki okumada eşitlenir).
    let can_sync = not_exporting().is_ok();
    let mut synced = 0;
    for (row, link) in rows.iter().zip(&links) {
        let Some(j) = *link else { continue };
        if !can_sync || row.matches(entries[j]) {
            continue;
        }
        if let Some(fresh) = row.apply(entries[j]) {
            store
                .save_exported_entry(&exported[j].id, &fresh, false, Some(&row.consultant))
                .map_err(err)?;
            synced += 1;
        }
    }
    let linked: std::collections::HashSet<usize> = links.iter().flatten().copied().collect();
    Ok(SheetRows {
        missing: (0..exported.len())
            .filter(|j| !linked.contains(j))
            .map(|j| exported[j].id.clone())
            .collect(),
        rows: rows
            .into_iter()
            .zip(links)
            .map(|(row, link)| SheetRowView {
                row,
                entry_id: link.map(|j| exported[j].id.clone()),
            })
            .collect(),
        synced,
    })
}

/// Dosyadaki satırın yeni değerleri Kum'un satır biçiminde (başlangıç, saat ve tür geçerli olmalı).
fn file_entry(row: FileRow) -> CmdResult<TimesheetEntry> {
    let (Some(start), Some(hours)) = (row.start, row.hours) else {
        return Err("Başlangıç ve saat dolu olmalı.".into());
    };
    if !(hours > 0.0 && hours <= 24.0) {
        return Err("saat 0 ile 24 arasında olmalı".into());
    }
    let kind = match row.kind.trim() {
        "Working" => timesheet::EntryKind::Working,
        "Online" => timesheet::EntryKind::Online,
        "F2F" => timesheet::EntryKind::F2F,
        k => return Err(format!("bilinmeyen tür: {k}")),
    };
    Ok(TimesheetEntry {
        date: row.date,
        start,
        hours,
        actual_hours: None,
        kind,
        details: row.details,
        party: row.party,
        project_id: String::new(),
        division: row.division,
        coverage: None,
    })
}

/// Dosyada Kum dışında girilmiş satırı değiştirir: `expect` satırın okunan hali, `row` yeni
/// değerleri. Tarih değiştiyse satır dosyada yeni gününe taşınır. Yazılan satırın numarası.
#[tauri::command]
pub async fn save_sheet_row(
    app: AppHandle,
    timesheet_id: String,
    expect: FileRow,
    row: FileRow,
) -> CmdResult<u32> {
    let _write = FILE_WRITES.lock().await;
    let (sheet, target) = FileTarget::load(&lock(&app.state::<Shared>().store), &timesheet_id)?;
    let entry = file_entry(row)?;
    target.update(&sheet.consultant, &expect, &entry).await
}

/// Silinen dosya satırını geri ekler (gününe, Kum'un aktarımıyla aynı kurallarla). Yazılan satırın
/// numarası.
#[tauri::command]
pub async fn restore_sheet_row(
    app: AppHandle,
    timesheet_id: String,
    row: FileRow,
) -> CmdResult<u32> {
    let _write = FILE_WRITES.lock().await;
    let (sheet, target) = FileTarget::load(&lock(&app.state::<Shared>().store), &timesheet_id)?;
    let entry = file_entry(row)?;
    target.insert(&sheet.consultant, &entry).await
}

/// Dosyada Kum dışında girilmiş satırı kaldırır (günün tek satırıysa boşaltılır).
#[tauri::command]
pub async fn delete_sheet_row(
    app: AppHandle,
    timesheet_id: String,
    expect: FileRow,
) -> CmdResult<()> {
    let _write = FILE_WRITES.lock().await;
    let (_, target) = FileTarget::load(&lock(&app.state::<Shared>().store), &timesheet_id)?;
    target.remove(&expect, None).await
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
/// satır silinir. Aktarılmış satırın dosyadaki satırı da güncellenir ya da kaldırılır. Silinen
/// satır sayısı.
#[tauri::command]
pub async fn refresh_timesheet_entries(app: AppHandle, ids: Vec<String>) -> CmdResult<usize> {
    // Aktarılmış satırlar dosyaya da yazılır: sıradaki yazmalar bitince okunsun.
    let _write = FILE_WRITES.lock().await;
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
    let (mut removed, remote) = {
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
        // Aktarılmış satırlar: (kayıt, yeni hali; işi kalmadıysa `None`, çizelgenin dosyası).
        let mut remote = Vec::new();
        for id in &ids {
            let Some(saved) = store.timesheet_entry(id).map_err(err)? else {
                continue;
            };
            let day = pieces.get(saved.entry.date)?;
            if let Some((saved, sheet, target)) = exported_entry(&store, Some(id))? {
                if timesheet::stale_hours(day, &saved.entry).is_some() {
                    let fresh = timesheet::refreshed(day, &saved.entry);
                    remote.push((saved, sheet, target, fresh));
                }
                continue;
            }
            if !store.refresh_timesheet_entry(id, day).map_err(err)? {
                removed += 1;
            }
        }
        (removed, remote)
    };
    if remote.is_empty() {
        return Ok(removed);
    }
    for (saved, sheet, target, fresh) in remote {
        let expect = kum_row(&saved, 0, &sheet);
        match fresh {
            Some(fresh) => {
                target.update(&sheet.consultant, &expect, &fresh).await?;
                lock(&app.state::<Shared>().store)
                    .save_exported_entry(&saved.id, &fresh, true, Some(&sheet.consultant))
                    .map_err(err)?;
            }
            None => {
                target.remove(&expect, Some(&saved.id)).await?;
                lock(&app.state::<Shared>().store)
                    .withdraw_exported_entry(&saved.id, false)
                    .map_err(err)?;
                removed += 1;
            }
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
