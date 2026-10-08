//! Satırları çizelgenin Excel dosyasına ya da Google Sheets tablosuna aktarma ve son
//! aktarımı geri alma.

use super::*;

/// Aktarım sürüyor: ikinci bir aktarım (çift tıklama) aynı kayıtları dosyaya ikinci kez
/// yazmasın diye reddedilir.
pub(super) static EXPORTING: AtomicBool = AtomicBool::new(false);
/// Geri alınabilecek son aktarım (yalnızca bu oturumda).
pub(super) static LAST_EXPORT: std::sync::Mutex<Option<LastExport>> = std::sync::Mutex::new(None);

pub(super) struct LastExport {
    ids: Vec<String>,
    target: ExportTarget,
}

pub(super) enum ExportTarget {
    Sheets {
        url: String,
        token: String,
    },
    /// Sheets API ile doğrudan.
    Api {
        id: String,
        auth: GoogleAuth,
    },
    /// Dosya aktarımdan sonra değiştiyse yedek geri yüklenmez (sonraki emek kaybolmasın).
    Excel {
        path: std::path::PathBuf,
        backup: std::path::PathBuf,
        modified: Option<std::time::SystemTime>,
    },
}

pub(super) fn modified(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Aktarım bayrağını bırakır (hata ya da panikte de).
pub(super) struct ExportGuard;

impl Drop for ExportGuard {
    fn drop(&mut self) {
        EXPORTING.store(false, Ordering::Release);
    }
}

/// Aktarım sürerken kayıtları değiştiren komutları reddeder: aktarım, yazdığı kayıtları sonunda
/// "aktarıldı" işaretler; arada değişen kayıt dosyaya eski haliyle yazılmış ya da hiç
/// yazılmamışken işaretlenirdi. Depo kilidi tutulurken çağrılmalı (aktarım gönderdiği kayıtları
/// aynı kilit altında okur).
pub(super) fn not_exporting() -> CmdResult<()> {
    if EXPORTING.load(Ordering::Acquire) {
        return Err("Zaman çizelgesi aktarılıyor; aktarım bitince tekrar dene.".into());
    }
    Ok(())
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
    let _write = FILE_WRITES.lock().await;
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
    let (sheet, file_target, ids, pending) = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let ctx = store.timesheet_context().map_err(err)?;
        let sheet = ctx
            .config
            .timesheet(&timesheet_id)
            .cloned()
            .ok_or(NO_SHEET)?;
        let file_target = FileTarget::of(
            &sheet,
            &ctx.config.sheet_token,
            &crate::google::load(&store),
        )?;
        // Kaydedilmiş satırlar veritabanından okunur (arayüzdeki kopya eski olabilir).
        let mut entries: Vec<(Option<String>, TimesheetEntry)> = Vec::new();
        for r in rows {
            let Some(id) = r.id else {
                // Az önce kaydedilen (ör. açıklaması yazılıp hemen gönderilen) canlı satır:
                // yazılan hali gönderilir, eski öneri onu ezmez.
                match store.saved_twin(&r.entry).map_err(err)? {
                    Some(saved) => entries.push((Some(saved.id), saved.entry)),
                    None => entries.push((None, r.entry)),
                }
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
        (sheet, file_target, ids, pending)
    };
    let rows: Vec<tracky_xlsx::Row> = pending.iter().map(|(_, e)| xlsx_row(e)).collect();
    let consultant = sheet.consultant.clone();
    let (exported, target) = match file_target {
        FileTarget::Api { .. } | FileTarget::Sheets { .. } => {
            let keyed: Vec<_> = ids.iter().cloned().zip(rows).collect();
            let (done, used) = file_target
                .run(move |target| match target {
                    FileTarget::Api { id, auth, .. } => with_google(auth, |t| {
                        tracky_xlsx::gsheets::append(t, id, &consultant, &keyed)
                    }),
                    FileTarget::Sheets { url, token } => {
                        tracky_xlsx::sheets::append(url, token, &consultant, &keyed)
                    }
                    FileTarget::Excel { .. } => unreachable!("aşağıda ayrıldı"),
                })
                .await?;
            let target = match used {
                FileTarget::Api { id, auth, .. } => ExportTarget::Api { id, auth },
                FileTarget::Sheets { url, token } => ExportTarget::Sheets { url, token },
                FileTarget::Excel { .. } => unreachable!("aşağıda ayrıldı"),
            };
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
        FileTarget::Excel { path: file } => {
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
                target: file.display().to_string(),
                sheets: false,
            };
            let target = ExportTarget::Excel {
                modified: modified(&file),
                path: file,
                backup: done.backup,
            };
            (exported, target)
        }
    };
    lock(&app.state::<Shared>().store)
        .mark_timesheet_exported(&ids, Utc::now(), &sheet.id, &sheet.consultant)
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
    let _write = FILE_WRITES.lock().await;
    let Some(last) = lock(&LAST_EXPORT).take() else {
        return Err("Geri alınacak aktarım yok.".into());
    };
    // Yeniden aktarılmamış sayılacak kayıtlar: tabloda bulunamayan satır (elle silinmiş ya da
    // aynısı zaten vardı diye atlanmış) geri alınmadı; o kayıt aktarılmış kalır.
    let (message, undone) = match &last.target {
        ExportTarget::Sheets { .. } | ExportTarget::Api { .. } => {
            let ids = last.ids.clone();
            let done = match &last.target {
                ExportTarget::Api { id, auth } => {
                    let (id, auth) = (id.clone(), auth.clone());
                    tauri::async_runtime::spawn_blocking(move || {
                        with_google(&auth, |t| tracky_xlsx::gsheets::undo(t, &id, &ids))
                    })
                    .await
                }
                ExportTarget::Sheets { url, token } => {
                    let (url, token) = (url.clone(), token.clone());
                    tauri::async_runtime::spawn_blocking(move || {
                        tracky_xlsx::sheets::undo(&url, &token, &ids)
                    })
                    .await
                }
                ExportTarget::Excel { .. } => unreachable!("yukarıda ayrıldı"),
            }
            .map_err(err)?;
            let done = match done {
                Ok(d) => d,
                Err(e) => {
                    *lock(&LAST_EXPORT) = Some(last);
                    return Err(err(e));
                }
            };
            let missing = if done.missing > 0 {
                format!(
                    " {} satır tabloda bulunamadı; onlar aktarılmış sayılmaya devam ediyor.",
                    done.missing
                )
            } else {
                String::new()
            };
            let message = format!(
                "Aktarım geri alındı: {} satır silindi, {} satır boşaltıldı.{missing}",
                done.removed, done.cleared
            );
            (message, done.undone)
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
            let message = "Aktarım geri alındı: Excel dosyası aktarım öncesi haline döndü.";
            (message.to_string(), last.ids.clone())
        }
    };
    lock(&app.state::<Shared>().store)
        .unmark_timesheet_exported(&undone)
        .map_err(err)?;
    Ok(message)
}
