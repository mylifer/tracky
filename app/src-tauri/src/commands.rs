//! Raporlar, kategoriler ve gizlilik ayarları için arayüz komutları.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tracky_core::search::SearchResult;
use tracky_core::suggest::Suggestions;
use tracky_core::trends::Trends;
use tracky_core::{
    Client, Goals, PrivacySettings, Report, Rule, RuleField, Tag, TagKind, UsageTotal,
};

use crate::edits::{Edited, UndoOp, record};
use crate::lock;
use crate::tracking::{Command, GOALS_KEY, Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// `start` (YYYY-MM-DD, yerel) gününden başlayan `days` günlük rapor. `until`
/// verilirse rapor o anda kesilir (süren dönemi önceki dönemin aynı noktasıyla kıyaslamak için).
#[tauri::command]
pub async fn get_report(
    app: AppHandle,
    start: String,
    days: u32,
    timeline: bool,
    until: Option<String>,
) -> CmdResult<Report> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let days = days.clamp(1, 366);
    let starts: Vec<_> = (0..days)
        .map(|i| local_midnight(first + Days::new(i.into())))
        .collect();
    let mut to = local_midnight(first + Days::new(days.into()));
    if let Some(until) = until {
        to = to.min(parse_time(&until)?.max(starts[0]));
    }
    lock(&app.state::<Shared>().store)
        .report(starts[0], to, &starts, timeline)
        .map_err(err)
}

/// `start` gününden başlayan `days` günde başlığında ya da uygulama adında `query` geçen süre.
#[tauri::command]
pub async fn search(
    app: AppHandle,
    query: String,
    start: String,
    days: u32,
) -> CmdResult<SearchResult> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let starts: Vec<_> = (0..days.clamp(1, 366))
        .map(|i| local_midnight(first + Days::new(i.into())))
        .collect();
    let to = local_midnight(first + Days::new(starts.len() as u64)).min(Utc::now());
    lock(&app.state::<Shared>().store)
        .search(&query, starts[0], to.max(starts[0]), &starts)
        .map_err(err)
}

/// Son `weeks` haftanın (pazartesiden; bu hafta dahil, şimdiye kadar) eğilimleri.
#[tauri::command]
pub async fn get_trends(app: AppHandle, weeks: u32) -> CmdResult<Trends> {
    let today = Local::now().date_naive();
    let this_week = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    let weeks = u64::from(weeks.clamp(1, 52));
    let mut bounds: Vec<_> = (0..weeks)
        .rev()
        .map(|i| local_midnight(this_week - Days::new(7 * i)))
        .collect();
    bounds.push(Utc::now().max(*bounds.last().expect("en az bir hafta")));
    lock(&app.state::<Shared>().store)
        .trends(&bounds)
        .map_err(err)
}

#[tauri::command]
pub async fn app_titles_between(
    app: AppHandle,
    app_id: String,
    start: String,
    days: u32,
) -> CmdResult<Vec<UsageTotal>> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let from = local_midnight(first);
    let to = local_midnight(first + Days::new(days.clamp(1, 366).into()));
    lock(&app.state::<Shared>().store)
        .title_totals(&app_id, from, to)
        .map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Taxonomy {
    tags: Vec<TagView>,
    /// Etkin kurallar (arşivdeki projelerinkiler hariç).
    rules: Vec<Rule>,
    clients: Vec<ClientView>,
    /// Proje → müşteri.
    project_clients: std::collections::HashMap<String, String>,
}

/// Etiket, arşiv durumu ve sözleşme bütçesiyle.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagView {
    #[serde(flatten)]
    tag: Tag,
    archived: bool,
    budget_days: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    #[serde(flatten)]
    client: Client,
    budget_days: Option<f64>,
}

#[tauri::command]
pub async fn get_taxonomy(app: AppHandle) -> CmdResult<Taxonomy> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut extras = store.tag_extras().map_err(err)?;
    let budgets = store.client_budgets().map_err(err)?;
    Ok(Taxonomy {
        tags: store
            .tags()
            .map_err(err)?
            .into_iter()
            .map(|tag| {
                let e = extras.remove(&tag.id).unwrap_or_default();
                TagView {
                    tag,
                    archived: e.archived_at.is_some(),
                    budget_days: e.budget_days,
                }
            })
            .collect(),
        rules: store.rules().map_err(err)?,
        clients: store
            .clients()
            .map_err(err)?
            .into_iter()
            .map(|client| ClientView {
                budget_days: budgets.get(&client.id).copied(),
                client,
            })
            .collect(),
        project_clients: store.project_clients().map_err(err)?,
    })
}

/// Projeyi arşivler (geçmişi korunur, yeni süre toplamaz); geri alma numarasını döndürür.
#[tauri::command]
pub async fn archive_project(app: AppHandle, id: String) -> CmdResult<u64> {
    let snap = lock(&app.state::<Shared>().store)
        .archive_project(&id)
        .map_err(err)?;
    Ok(record(
        &app,
        vec![
            UndoOp::Archived { id, previous: None },
            UndoOp::Sessions(snap),
        ],
    ))
}

/// Projeyi arşivden çıkarır; geri alma numarasını döndürür.
#[tauri::command]
pub async fn unarchive_project(app: AppHandle, id: String) -> CmdResult<u64> {
    let previous = lock(&app.state::<Shared>().store)
        .unarchive_project(&id)
        .map_err(err)?;
    Ok(record(&app, vec![UndoOp::Archived { id, previous }]))
}

/// Projenin sözleşme bütçesi (adam-gün; `null` kaldırır).
#[tauri::command]
pub async fn set_project_budget(app: AppHandle, id: String, days: Option<f64>) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .set_project_budget(&id, days)
        .map_err(err)
}

/// Müşterinin sözleşme bütçesi (adam-gün; `null` kaldırır).
#[tauri::command]
pub async fn set_client_budget(app: AppHandle, id: String, days: Option<f64>) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .set_client_budget(&id, days)
        .map_err(err)
}

/// Bütçeler ve bugüne kadar harcanan süre.
#[tauri::command]
pub async fn get_budgets(app: AppHandle) -> CmdResult<tracky_core::budget::Budgets> {
    lock(&app.state::<Shared>().store)
        .budgets(Utc::now())
        .map_err(err)
}

/// Müşteri ekler (`id` yoksa) ya da adını değiştirir; kaydedileni döndürür.
#[tauri::command]
pub async fn save_client(app: AppHandle, id: Option<String>, name: String) -> CmdResult<Client> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let client = Client {
        id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: name.trim().to_string(),
    };
    let position = store.clients().map_err(err)?.len() as i64;
    store.upsert_client(&client, position).map_err(err)?;
    Ok(client)
}

/// Müşteriyi siler; projeleri kalır, müşterisiz olur.
#[tauri::command]
pub async fn delete_client(app: AppHandle, id: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .delete_client(&id)
        .map_err(err)
}

/// Projeyi müşteriye bağlar (`null`: müşterisiz).
#[tauri::command]
pub async fn set_project_client(
    app: AppHandle,
    project_id: String,
    client_id: Option<String>,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .set_project_client(&project_id, client_id.as_deref())
        .map_err(err)
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
pub async fn save_tag(app: AppHandle, tag: TagInput) -> CmdResult<Tag> {
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

/// Siler; geri alma numarasını döndürür.
#[tauri::command]
pub async fn delete_tag(app: AppHandle, id: String) -> CmdResult<u64> {
    let at = lock(&app.state::<Shared>().store)
        .delete_tag(&id)
        .map_err(err)?;
    Ok(record(&app, vec![UndoOp::DeletedTag { id, at }]))
}

/// Kural ekler; geri alma numarasını döndürür.
#[tauri::command]
pub async fn add_rule(
    app: AppHandle,
    tag_id: String,
    field: RuleField,
    pattern: String,
) -> CmdResult<u64> {
    let id = uuid::Uuid::new_v4().to_string();
    lock(&app.state::<Shared>().store)
        .upsert_rule(&Rule {
            id: id.clone(),
            tag_id,
            field,
            pattern,
        })
        .map_err(err)?;
    Ok(record(&app, vec![UndoOp::AddedRule(id)]))
}

/// Kuralı siler; geri alma numarasını döndürür.
#[tauri::command]
pub async fn delete_rule(app: AppHandle, id: String) -> CmdResult<u64> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let rule = store.rule_by_id(&id).map_err(err)?;
    store.delete_rule(&id).map_err(err)?;
    drop(store);
    Ok(record(
        &app,
        rule.map(UndoOp::DeletedRule).into_iter().collect(),
    ))
}

#[tauri::command]
pub async fn assign_app_category(
    app: AppHandle,
    app_id: String,
    tag_id: Option<String>,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .assign_app_category(&app_id, tag_id.as_deref())
        .map_err(err)
}

fn parse_time(s: &str) -> CmdResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("geçersiz zaman {s}: {e}"))
}

/// `[start, end)` aralığında geri alınabilir bir düzenleme: önce durum saklanır.
fn range_edit(
    app: &AppHandle,
    start: &str,
    end: &str,
    edit: impl FnOnce(
        &tracky_core::Store,
        DateTime<Utc>,
        DateTime<Utc>,
    ) -> tracky_core::store::Result<usize>,
) -> CmdResult<Edited> {
    let (from, to) = (parse_time(start)?, parse_time(end)?);
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let snap = store.snapshot_range(from, to).map_err(err)?;
    let changed = edit(&store, from, to).map_err(err)?;
    drop(store);
    Ok(Edited {
        changed,
        undo: record(app, vec![UndoOp::Sessions(snap)]),
    })
}

/// Takvimdeki bir bloğu (aralıktaki oturumları) kategoriye atar; `None` kurallara döndürür.
#[tauri::command]
pub async fn set_range_category(
    app: AppHandle,
    start: String,
    end: String,
    category_id: Option<String>,
) -> CmdResult<Edited> {
    range_edit(&app, &start, &end, |s, from, to| {
        s.set_category_between(from, to, category_id.as_deref())
    })
}

/// Takvimdeki bir bloğu ya da aralığı projeye atar; `None` kurallara döndürür.
#[tauri::command]
pub async fn set_range_project(
    app: AppHandle,
    start: String,
    end: String,
    project_id: Option<String>,
) -> CmdResult<Edited> {
    range_edit(&app, &start, &end, |s, from, to| {
        s.set_project_between(from, to, project_id.as_deref())
    })
}

#[tauri::command]
pub async fn delete_range(app: AppHandle, start: String, end: String) -> CmdResult<Edited> {
    range_edit(&app, &start, &end, |s, from, to| s.delete_between(from, to))
}

#[tauri::command]
pub async fn add_manual_entry(
    app: AppHandle,
    label: String,
    start: String,
    end: String,
    category_id: Option<String>,
    project_id: Option<String>,
) -> CmdResult<Edited> {
    range_edit(&app, &start, &end, |s, from, to| {
        s.add_manual_session(
            &label,
            from,
            to,
            category_id.as_deref(),
            project_id.as_deref(),
        )
        .map(|_| 1)
    })
}

/// Başlıklardan bulunan projeler ve tanınan uygulama/sitelerden kategori önerileri.
#[tauri::command]
pub async fn get_suggestions(app: AppHandle) -> CmdResult<Suggestions> {
    lock(&app.state::<Shared>().store)
        .suggestions(Utc::now())
        .map_err(err)
}

#[tauri::command]
pub async fn accept_project_suggestion(app: AppHandle, name: String) -> CmdResult<Tag> {
    lock(&app.state::<Shared>().store)
        .accept_project_suggestion(&name)
        .map_err(err)
}

#[tauri::command]
pub async fn accept_category_suggestion(
    app: AppHandle,
    field: RuleField,
    pattern: String,
    category_id: String,
) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .accept_category_suggestion(field, &pattern, &category_id)
        .map_err(err)
}

#[tauri::command]
pub async fn dismiss_suggestion(app: AppHandle, key: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .dismiss_suggestion(&key)
        .map_err(err)
}

#[tauri::command]
pub async fn known_apps(app: AppHandle) -> CmdResult<Vec<UsageTotal>> {
    lock(&app.state::<Shared>().store)
        .known_apps(200)
        .map_err(err)
}

#[tauri::command]
pub async fn get_privacy(app: AppHandle) -> CmdResult<PrivacySettings> {
    lock(&app.state::<Shared>().store)
        .privacy_settings()
        .map_err(err)
}

/// Duraklatma durumu ayrı yönetilir (menü çubuğu); buradan değiştirilmez.
#[tauri::command]
pub async fn save_privacy(app: AppHandle, settings: PrivacySettings) -> CmdResult<()> {
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

#[tauri::command]
pub async fn get_goals(app: AppHandle) -> CmdResult<Goals> {
    Ok(lock(&app.state::<Shared>().store)
        .setting::<Goals>(GOALS_KEY)
        .map_err(err)?
        .unwrap_or_default())
}

#[tauri::command]
pub async fn save_goals(app: AppHandle, goals: Goals) -> CmdResult<()> {
    let goals = Goals {
        daily_hours: goals.daily_hours.clamp(0.0, 24.0),
        break_after_minutes: goals.break_after_minutes.map(|m| m.clamp(10, 240)),
        day_summary_at: goals.day_summary_at.map(|m| m.min(24 * 60 - 1)),
        export_reminder_at: goals.export_reminder_at.map(|m| m.min(24 * 60 - 1)),
        limits: {
            // Kategori başına tek limit; geçersiz süreler atılır.
            let mut seen = std::collections::HashSet::new();
            goals
                .limits
                .into_iter()
                .filter(|l| {
                    (1..=24 * 60).contains(&l.minutes) && seen.insert(l.category_id.clone())
                })
                .collect()
        },
        project_goals: {
            // Proje başına tek hedef; haftada en çok 100 saat.
            let mut seen = std::collections::HashSet::new();
            goals
                .project_goals
                .into_iter()
                .filter(|g| {
                    (1..=100 * 60).contains(&g.minutes) && seen.insert(g.project_id.clone())
                })
                .collect()
        },
        ..goals
    };
    lock(&app.state::<Shared>().store)
        .save_setting(GOALS_KEY, &goals)
        .map_err(err)?;
    app.state::<crate::Worker>()
        .tx
        .send(Command::SetGoals(goals))
        .map_err(err)
}

/// Tüm kayıtları İndirilenler klasörüne CSV olarak yazar ve dosyayı gösterir.
#[tauri::command]
pub async fn export_csv(app: AppHandle) -> CmdResult<String> {
    let csv = lock(&app.state::<Shared>().store)
        .export_csv()
        .map_err(err)?;
    save_download(&app, "kum", &csv)
}

/// Aramayla eşleşen oturumları (`start`'tan itibaren `days` gün) İndirilenler'e CSV yazar.
#[tauri::command]
pub async fn export_search(
    app: AppHandle,
    query: String,
    start: String,
    days: u32,
) -> CmdResult<String> {
    let first = NaiveDate::parse_from_str(&start, "%Y-%m-%d").map_err(err)?;
    let from = local_midnight(first);
    let to = local_midnight(first + Days::new(days.clamp(1, 366).into())).min(Utc::now());
    let csv = lock(&app.state::<Shared>().store)
        .export_search_csv(&query, from, to.max(from))
        .map_err(err)?;
    save_download(&app, &format!("kum-{}", file_slug(&query)), &csv)
}

/// Dosya adına uygun kısa ad: harf ve rakamlar, gerisi tire ("Müşteri X" → "müşteri-x").
fn file_slug(s: &str) -> String {
    let slug: String = tracky_core::search::fold(s)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect();
    if slug.is_empty() {
        "arama".into()
    } else {
        slug
    }
}

/// İndirilenler'e (yoksa ev dizinine) `ad-tarih.csv` yazar ve dosyayı gösterir.
fn save_download(app: &AppHandle, name: &str, csv: &str) -> CmdResult<String> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)?;
    let path = dir.join(format!(
        "{name}-{}.csv",
        Local::now().format("%Y-%m-%d-%H%M")
    ));
    std::fs::write(&path, csv).map_err(err)?;
    reveal(&path);
    Ok(path.display().to_string())
}

/// Dosyayı Finder / Gezgin'de seçili gösterir (başarısızlık önemsiz).
pub(crate) fn reveal(path: &std::path::Path) {
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

#[cfg(test)]
mod tests {
    use super::file_slug;

    #[test]
    fn file_slugs_are_safe() {
        assert_eq!(file_slug("Müşteri X"), "müşteri-x");
        assert_eq!(file_slug("İSTANBUL / Proje #2"), "istanbul-proje-2");
        assert_eq!(file_slug("../../etc"), "etc");
        assert_eq!(file_slug("  ***  "), "arama");
    }
}
