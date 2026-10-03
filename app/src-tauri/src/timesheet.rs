//! Zaman çizelgesi komutları: günlük kayıt önerileri, onaylama ve düzenleme, şablonu
//! içe aktarma ve kayıtları kullanıcının Excel dosyasına ekleme.

use chrono::{Days, NaiveDate, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::timesheet::{ProjectMapping, TimesheetConfig, TimesheetEntry};
use tracky_core::{Rule, RuleField, Tag, TagKind};

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Geçmiş açıklamalar (otomatik tamamlama), şablondan içe aktarılır.
const DETAILS_KEY: &str = "timesheet_details";
const MAX_DETAILS: usize = 300;

/// Bir günün kayıtları: onaylandıysa kaydedilenler, yoksa canlı öneri.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    date: NaiveDate,
    approved: bool,
    entries: Vec<EntryView>,
    /// Bir projeye atanmamış takip edilen süre (saniye): gözden geçirilecek.
    unassigned_seconds: i64,
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
pub async fn save_timesheet_config(app: AppHandle, config: TimesheetConfig) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .save_timesheet_config(&config)
        .map_err(err)
}

/// `start` gününden itibaren `days` günün kayıtları.
#[tauri::command]
pub async fn timesheet_days(app: AppHandle, start: String, days: u32) -> CmdResult<Vec<Day>> {
    let first = parse_date(&start)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    (0..days.clamp(1, 62))
        .map(|i| {
            let date = first + Days::new(i.into());
            let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
            let saved = store.timesheet_entries(date, date).map_err(err)?;
            let report = store.report(from, to, &[from], false).map_err(err)?;
            let assigned: i64 = report
                .projects
                .iter()
                .filter(|b| b.id.is_some())
                .map(|b| b.seconds)
                .sum();
            let entries = if saved.is_empty() {
                store
                    .propose_timesheet(from, to)
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
                unassigned_seconds: (report.total_seconds - assigned).max(0),
            })
        })
        .collect()
}

/// Günün önerilerini kaydeder (onaylar); onaylı günde "yeniden öner" olarak da kullanılır.
/// Excel'e aktarılmış kayıtlar korunur.
#[tauri::command]
pub async fn approve_timesheet_day(app: AppHandle, date: String) -> CmdResult<()> {
    let date = parse_date(&date)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
    let proposed = store.propose_timesheet(from, to).map_err(err)?;
    store.replace_timesheet_day(date, &proposed).map_err(err)
}

#[tauri::command]
pub async fn save_timesheet_entry(
    app: AppHandle,
    id: Option<String>,
    entry: TimesheetEntry,
) -> CmdResult<String> {
    lock(&app.state::<Shared>().store)
        .save_timesheet_entry(id.as_deref(), &entry)
        .map_err(err)
}

#[tauri::command]
pub async fn delete_timesheet_entry(app: AppHandle, id: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .delete_timesheet_entry(&id)
        .map_err(err)
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
    if let Some(c) = template.company {
        config.company = c;
    }
    if let Some(c) = template.consultant {
        config.consultant = c;
    }
    if let Some(p) = template.parties.first() {
        config.default_party = p.clone();
    }
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
        if !config.projects.iter().any(|m| m.project_id == id) {
            config.projects.push(ProjectMapping {
                project_id: id,
                division: division.clone(),
                party: None,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    rows: usize,
    filled: usize,
    inserted: usize,
    backup: String,
    path: String,
}

/// `start`'tan itibaren `days` gündeki onaylı ve aktarılmamış kayıtları Excel dosyasına ekler.
#[tauri::command]
pub async fn export_timesheet(app: AppHandle, start: String, days: u32) -> CmdResult<Exported> {
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
    let path = config
        .file_path
        .clone()
        .ok_or("Önce Excel dosyasını seç (Zaman çizelgesi ayarları)")?;
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
    let done = {
        let (path, consultant) = (std::path::PathBuf::from(&path), config.consultant.clone());
        tauri::async_runtime::spawn_blocking(move || tracky_xlsx::append(&path, &consultant, &rows))
            .await
            .map_err(err)?
            .map_err(err)?
    };
    let ids: Vec<String> = pending.iter().map(|e| e.id.clone()).collect();
    lock(&app.state::<Shared>().store)
        .mark_timesheet_exported(&ids, Utc::now())
        .map_err(err)?;
    Ok(Exported {
        rows: ids.len(),
        filled: done.filled,
        inserted: done.inserted,
        backup: done.backup.display().to_string(),
        path,
    })
}
