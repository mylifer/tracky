//! Raporlar, kategoriler ve gizlilik ayarları için arayüz komutları.

use chrono::{DateTime, Days, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tracky_core::{PrivacySettings, Report, Rule, RuleField, Tag, TagKind, UsageTotal};

use crate::lock;
use crate::tracking::{Command, Shared};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Yerel gece yarısı (yaz saati geçişinde ilk geçerli an).
fn local_midnight(date: NaiveDate) -> CmdResult<DateTime<Utc>> {
    date.and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).earliest())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| format!("{date} için yerel saat hesaplanamadı"))
}

/// `start` (YYYY-MM-DD, yerel) gününden başlayan `days` günlük rapor.
#[tauri::command]
pub fn get_report(app: AppHandle, start: String, days: u32, timeline: bool) -> CmdResult<Report> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let days = days.clamp(1, 366);
    let starts = (0..days)
        .map(|i| local_midnight(first + Days::new(i.into())))
        .collect::<CmdResult<Vec<_>>>()?;
    let to = local_midnight(first + Days::new(days.into()))?;
    lock(&app.state::<Shared>().store)
        .report(starts[0], to, &starts, timeline)
        .map_err(err)
}

#[tauri::command]
pub fn app_titles_between(
    app: AppHandle,
    app_id: String,
    start: String,
    days: u32,
) -> CmdResult<Vec<UsageTotal>> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let from = local_midnight(first)?;
    let to = local_midnight(first + Days::new(days.clamp(1, 366).into()))?;
    lock(&app.state::<Shared>().store)
        .title_totals(&app_id, from, to)
        .map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Taxonomy {
    tags: Vec<Tag>,
    rules: Vec<Rule>,
}

#[tauri::command]
pub fn get_taxonomy(app: AppHandle) -> CmdResult<Taxonomy> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    Ok(Taxonomy {
        tags: store.tags().map_err(err)?,
        rules: store.rules().map_err(err)?,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInput {
    id: Option<String>,
    kind: TagKind,
    name: String,
    color: u8,
}

/// Yeni etiket için kimlik üretir; kaydedilen etiketi döndürür.
#[tauri::command]
pub fn save_tag(app: AppHandle, tag: TagInput) -> CmdResult<Tag> {
    if tag.name.trim().is_empty() {
        return Err("Ad boş olamaz".into());
    }
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let position = store.tags().map_err(err)?.len() as i64;
    let tag = Tag {
        id: tag.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        kind: tag.kind,
        name: tag.name.trim().to_string(),
        color: tag.color,
    };
    store.upsert_tag(&tag, position).map_err(err)?;
    Ok(tag)
}

#[tauri::command]
pub fn delete_tag(app: AppHandle, id: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .delete_tag(&id)
        .map_err(err)
}

#[tauri::command]
pub fn add_rule(
    app: AppHandle,
    tag_id: String,
    field: RuleField,
    pattern: String,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .upsert_rule(&Rule {
            id: uuid::Uuid::new_v4().to_string(),
            tag_id,
            field,
            pattern,
        })
        .map_err(err)
}

#[tauri::command]
pub fn delete_rule(app: AppHandle, id: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .delete_rule(&id)
        .map_err(err)
}

#[tauri::command]
pub fn assign_app_category(
    app: AppHandle,
    app_id: String,
    tag_id: Option<String>,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .assign_app_category(&app_id, tag_id.as_deref())
        .map_err(err)
}

#[tauri::command]
pub fn known_apps(app: AppHandle) -> CmdResult<Vec<UsageTotal>> {
    lock(&app.state::<Shared>().store)
        .known_apps(200)
        .map_err(err)
}

#[tauri::command]
pub fn get_privacy(app: AppHandle) -> CmdResult<PrivacySettings> {
    lock(&app.state::<Shared>().store)
        .privacy_settings()
        .map_err(err)
}

/// Duraklatma durumu ayrı yönetilir (menü çubuğu); buradan değiştirilmez.
#[tauri::command]
pub fn save_privacy(app: AppHandle, settings: PrivacySettings) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let saved = {
        let store = lock(&shared.store);
        let paused = store.privacy_settings().map_err(err)?.paused;
        let mut settings = settings;
        settings.paused = paused;
        for list in [
            &mut settings.excluded_apps,
            &mut settings.hidden_title_apps,
            &mut settings.title_suffixes,
        ] {
            list.retain(|a| !a.trim().is_empty());
            list.sort_unstable();
            list.dedup();
        }
        store.save_privacy_settings(&settings).map_err(err)?;
        settings
    };
    app.state::<crate::Worker>()
        .tx
        .send(Command::SetPrivacy(saved))
        .map_err(err)
}

/// Tüm kayıtları İndirilenler klasörüne CSV olarak yazar ve dosyayı gösterir.
#[tauri::command]
pub fn export_csv(app: AppHandle) -> CmdResult<String> {
    let csv = lock(&app.state::<Shared>().store)
        .export_csv()
        .map_err(err)?;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)?;
    let path = dir.join(format!("kum-{}.csv", Local::now().format("%Y-%m-%d-%H%M")));
    std::fs::write(&path, csv).map_err(err)?;
    reveal(&path);
    Ok(path.display().to_string())
}

/// Dosyayı Finder / Gezgin'de seçili gösterir (başarısızlık önemsiz).
fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}
