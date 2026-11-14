//! Eski kayıtların sadeleştirilmesi (Ayarlar → Veriler): seçilen süreden eski oturumların
//! pencere başlığı ve adres yolu silinir; süreler, uygulama, alan adı, kategori ve proje kalır
//! ([`tracky_core::Store::prune_details`]). Varsayılan kapalı; açılınca önce yedek alınır,
//! sonra günde bir kez çalışır. Ayar bu bilgisayara özgüdür (eşitlenmez); sadeleştirilen
//! oturumlar ise eşitlenir, öteki bilgisayarlarda da başlıksız görünür.

use chrono::{DateTime, Months, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::lock;
use crate::tracking::Shared;

type CmdResult<T> = Result<T, String>;

const KEY: &str = "retention";
/// Seçilebilen süreler (ay).
const CHOICES: &[u32] = &[6, 12, 24];
/// Düzenli sadeleştirme en sık bu aralıkla.
const EVERY: chrono::Duration = chrono::Duration::hours(24);
/// Bu kadar satır değişince veritabanı sıkıştırılır (yer gerçekten boşalsın).
const COMPACT_AFTER: usize = 1_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Retention {
    /// Bundan eski kayıtlar sadeleştirilir; `None`: kapalı.
    months: Option<u32>,
    last_run: Option<DateTime<Utc>>,
    /// Son çalışmada sadeleştirilen oturum sayısı.
    last_count: usize,
}

fn load(app: &AppHandle) -> Retention {
    lock(&app.state::<Shared>().store)
        .setting(KEY)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn save(app: &AppHandle, r: &Retention) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .save_setting(KEY, r)
        .map_err(|e| e.to_string())
}

/// Sınır: bugünden `months` ay önce.
fn cutoff(months: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    now.checked_sub_months(Months::new(months))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

/// Sadeleştirir ve sonucu kaydeder; değişen oturum sayısını döndürür.
fn run(app: &AppHandle, mut r: Retention) -> CmdResult<Retention> {
    let Some(months) = r.months else {
        return Ok(r);
    };
    let now = Utc::now();
    let count = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let count = store
            .prune_details(cutoff(months, now))
            .map_err(|e| e.to_string())?;
        if count >= COMPACT_AFTER
            && let Err(e) = store.compact()
        {
            log_error!("veritabanı sıkıştırılamadı: {e}");
        }
        count
    };
    if count > 0 {
        log_info!("{count} eski oturum sadeleştirildi ({months} aydan eski)");
        crate::sync::soon(app);
    }
    r.last_run = Some(now);
    r.last_count = count;
    save(app, &r)?;
    Ok(r)
}

/// Yedek döngüsünden (saatte bir) çağrılır: açıksa ve son çalışma bir günden eskiyse çalışır.
pub fn run_due(app: &AppHandle) {
    let r = load(app);
    let due = r.months.is_some() && r.last_run.is_none_or(|at| Utc::now() - at >= EVERY);
    if due && let Err(e) = run(app, r) {
        log_error!("eski kayıtlar sadeleştirilemedi: {e}");
    }
}

#[tauri::command]
pub async fn get_retention(app: AppHandle) -> Retention {
    load(&app)
}

/// Süreyi değiştirir (`None`: kapatır). Açılınca ya da süre kısalınca önce yedek alınır,
/// sonra hemen sadeleştirilir.
#[tauri::command]
pub async fn set_retention(app: AppHandle, months: Option<u32>) -> CmdResult<Retention> {
    if months.is_some_and(|m| !CHOICES.contains(&m)) {
        return Err("Geçersiz süre.".into());
    }
    let prev = load(&app);
    let next = Retention {
        months,
        ..prev.clone()
    };
    save(&app, &next)?;
    let tighter = match (prev.months, months) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(a), Some(b)) => b < a,
    };
    if !tighter {
        return Ok(next);
    }
    tauri::async_runtime::spawn_blocking(move || {
        // Geri alınamaz: önce yedek.
        crate::backup::backup_now_inner(&app)?;
        run(&app, next)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cutoff_is_calendar_months_back() {
        let now = Utc.with_ymd_and_hms(2026, 11, 14, 10, 0, 0).unwrap();
        assert_eq!(
            cutoff(6, now),
            Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap()
        );
        assert_eq!(
            cutoff(24, now),
            Utc.with_ymd_and_hms(2024, 11, 14, 10, 0, 0).unwrap()
        );
    }

    #[test]
    fn older_settings_load_as_off() {
        let r: Retention = serde_json::from_str("{}").unwrap();
        assert_eq!(r.months, None);
    }
}
