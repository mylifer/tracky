//! Aylık müşteri raporu komutları: ayın proje × gün saat tablosu ve Excel'e aktarımı.

use chrono::{Datelike, Days, Local, NaiveDate};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tracky_core::Store;
use tracky_core::client_report::{ClientReport, ReportSource};
use tracky_xlsx::report::{Matrix, MatrixRow};

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

const MONTHS: [&str; 12] = [
    "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim",
    "Kasım", "Aralık",
];

/// `month` (ayın herhangi bir günü, `YYYY-MM-DD`) → ayın günleri.
fn month_days(month: &str) -> CmdResult<Vec<NaiveDate>> {
    let any = NaiveDate::parse_from_str(month, "%Y-%m-%d").map_err(err)?;
    let first = any.with_day(1).ok_or("geçersiz ay")?;
    Ok(first
        .iter_days()
        .take_while(|d| d.month() == first.month())
        .collect())
}

fn build(
    store: &Store,
    month: &str,
    client: Option<&str>,
    source: Option<ReportSource>,
) -> CmdResult<ClientReport> {
    let days = month_days(month)?;
    let mut starts: Vec<_> = days.iter().map(|d| local_midnight(*d)).collect();
    let last = *days.last().ok_or("geçersiz ay")?;
    starts.push(local_midnight(last + Days::new(1)));
    store
        .client_report(days, &starts, client, source)
        .map_err(err)
}

/// Ayın raporu. `client` verilmezse tüm müşteriler; `source` verilmezse ayda zaman çizelgesi
/// kaydı varsa o, yoksa takip edilen süre.
#[tauri::command]
pub async fn client_report(
    app: AppHandle,
    month: String,
    client: Option<String>,
    source: Option<ReportSource>,
) -> CmdResult<ClientReport> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    build(&store, &month, client.as_deref(), source)
}

/// Raporu kaydetme penceresinde seçilen Excel dosyasına yazar; vazgeçilirse `None`.
#[tauri::command]
pub async fn export_client_report(
    app: AppHandle,
    month: String,
    client: Option<String>,
    source: Option<ReportSource>,
) -> CmdResult<Option<String>> {
    let (report, client_name, consultant) = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let report = build(&store, &month, client.as_deref(), source)?;
        let name = match &client {
            Some(id) => store
                .clients()
                .map_err(err)?
                .into_iter()
                .find(|c| &c.id == id)
                .map(|c| c.name),
            None => None,
        };
        let consultant = store.timesheet_config().map_err(err)?.consultant;
        (report, name, consultant)
    };
    let first = *report.days.first().ok_or("geçersiz ay")?;
    let period = format!("{} {}", MONTHS[first.month0() as usize], first.year());
    let who = client_name.as_deref().unwrap_or("Tüm müşteriler");
    // Müşteri adında dosya adına giremeyen karakterler olabilir.
    let safe: String = who
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    let file_name = format!("{} {}.xlsx", safe.trim(), first.format("%Y-%m"));
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Müşteri raporunu kaydet")
            .set_file_name(file_name)
            .add_filter("Excel", &["xlsx"])
            .blocking_save_file()
    })
    .await
    .map_err(err)?;
    let Some(mut path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension("xlsx");
    }
    let mut subtitle = match report.source {
        ReportSource::Timesheet => "Kaynak: zaman çizelgesi kayıtları".to_string(),
        ReportSource::Tracked => "Kaynak: takip edilen süre".to_string(),
    };
    if !consultant.trim().is_empty() {
        subtitle = format!("{} · {subtitle}", consultant.trim());
    }
    subtitle = format!(
        "{subtitle} · Oluşturuldu: {}",
        Local::now().format("%d.%m.%Y")
    );
    let matrix = Matrix {
        title: format!("{who} · {period}"),
        subtitle,
        days: report.days,
        rows: report
            .rows
            .into_iter()
            .map(|r| MatrixRow {
                client: r.client.unwrap_or_default(),
                project: r.project,
                hours: r.hours,
            })
            .collect(),
    };
    tracky_xlsx::report::write_matrix(&path, &matrix).map_err(err)?;
    crate::commands::reveal(&path);
    Ok(Some(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_days_cover_the_whole_month() {
        let days = month_days("2028-02-15").unwrap();
        assert_eq!(days.len(), 29);
        assert_eq!(days[0], NaiveDate::from_ymd_opt(2028, 2, 1).unwrap());
        assert!(month_days("şubat").is_err());
    }
}
