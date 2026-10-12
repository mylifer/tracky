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

/// Sürmekte olan aktarımın kaydı (yalnızca bu cihazda): uygulama dosyaya yazdıktan sonra,
/// satırları "gönderildi" işaretleyemeden kapanırsa (çökme, kapatma, veritabanı hatası)
/// bir sonraki açılışta ya da aktarımda hangi satırların gerçekten yazıldığına bakılır.
const INTERRUPTED_KEY: &str = "timesheet_export_inflight";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Interrupted {
    sheet_id: String,
    consultant: String,
    /// Excel dosyası; Sheets'te `None` (orada satırlar kimlikle işaretli, ikinci kez yazılmaz).
    excel: Option<std::path::PathBuf>,
    ids: Vec<String>,
    at: chrono::DateTime<Utc>,
}

fn save_interrupted(app: &AppHandle, journal: Option<&Interrupted>) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .save_setting(INTERRUPTED_KEY, &journal)
        .map_err(err)
}

/// Yarıda kalmış aktarımı çözer: Excel dosyasına yazılmış ama Kum'da gönderilmemiş görünen
/// satırlar "gönderildi" işaretlenir (yeniden gönderilip dosyada ikilenmesin). Dosyada
/// olmayanlar gönderilmemiş kalır. Sheets'te bir şey gerekmez: tablo, kimliğiyle yazılmış
/// satırı ikinci kez yazmaz. Dosya okunamazsa kayıt kalır ve hata döner: aktarım, önce
/// hangi satırların yazıldığı bilinmeden yapılmasın.
async fn recover_interrupted(app: &AppHandle) -> CmdResult<()> {
    let journal = lock(&app.state::<Shared>().store)
        .setting::<Option<Interrupted>>(INTERRUPTED_KEY)
        .map_err(err)?
        .flatten();
    let Some(journal) = journal else {
        return Ok(());
    };
    if let Some(path) = journal.excel.clone() {
        let pending = interrupted_pending(&lock(&app.state::<Shared>().store), &journal)?;
        if !pending.is_empty() {
            let rows: Vec<tracky_xlsx::Row> = pending.iter().map(|(_, e)| xlsx_row(e)).collect();
            let consultant = journal.consultant.clone();
            let found = tauri::async_runtime::spawn_blocking(move || {
                tracky_xlsx::contains(&path, &consultant, &rows)
            })
            .await
            .map_err(err)?
            .map_err(|e| {
                format!(
                    "Önceki aktarım yarıda kalmıştı; dosyaya hangi satırların yazıldığı okunamadı: {e}"
                )
            })?;
            let written = settle_interrupted(
                &lock(&app.state::<Shared>().store),
                &journal,
                &pending,
                &found,
            )?;
            log_info!(
                "yarıda kalan aktarım: {} satırdan {written}'i dosyada bulundu, gönderildi sayıldı",
                pending.len()
            );
        }
    }
    save_interrupted(app, None)
}

/// Yarıda kalan aktarımın hâlâ gönderilmemiş görünen satırları (silinen ya da bu arada
/// gönderilen dışarıda kalır).
fn interrupted_pending(
    store: &Store,
    journal: &Interrupted,
) -> CmdResult<Vec<(String, TimesheetEntry)>> {
    let mut pending = Vec::new();
    for id in &journal.ids {
        if let Some(saved) = store
            .timesheet_entry(id)
            .map_err(err)?
            .filter(|s| !s.dismissed && s.exported_at.is_none())
        {
            pending.push((id.clone(), saved.entry));
        }
    }
    Ok(pending)
}

/// `found[i]`: `pending[i]` dosyada var. Bulunanlar aktarımın zamanıyla gönderildi işaretlenir;
/// sayıları döner.
fn settle_interrupted(
    store: &Store,
    journal: &Interrupted,
    pending: &[(String, TimesheetEntry)],
    found: &[bool],
) -> CmdResult<usize> {
    let written: Vec<String> = pending
        .iter()
        .zip(found)
        .filter(|(_, found)| **found)
        .map(|((id, _), _)| id.clone())
        .collect();
    if !written.is_empty() {
        store
            .mark_timesheet_exported(&written, journal.at, &journal.sheet_id, &journal.consultant)
            .map_err(err)?;
    }
    Ok(written.len())
}

/// Açılışta: önceki oturumda yarıda kalan aktarım varsa çözer (aktarımla aynı dosya kilidiyle).
pub(crate) async fn recover_interrupted_export(app: &AppHandle) {
    let _write = FILE_WRITES.lock().await;
    if let Err(e) = recover_interrupted(app).await {
        log_error!("yarıda kalan aktarım çözülemedi: {e}");
    }
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
    // Önceki aktarım yarıda kaldıysa önce o çözülür: dosyaya yazılmış satırlar ikinci kez gitmesin.
    recover_interrupted(&app).await?;
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
        for (id, e) in &entries {
            let day = pieces.get(e.date)?;
            // Kaydedilmiş satırda aynı işi kapsayan, önce gelen satırlar da sayılır.
            let saved = id
                .as_deref()
                .map(|id| store.timesheet_entry(id))
                .transpose()
                .map_err(err)?
                .flatten();
            let hours = match &saved {
                Some(saved) => store.stale_hours(day, saved).map_err(err)?,
                None => timesheet::stale_hours(day, e),
            };
            if hours.is_some() {
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
    save_interrupted(
        &app,
        Some(&Interrupted {
            sheet_id: sheet.id.clone(),
            consultant: sheet.consultant.clone(),
            excel: match &file_target {
                FileTarget::Excel { path } => Some(path.clone()),
                _ => None,
            },
            ids: ids.clone(),
            at: Utc::now(),
        }),
    )?;
    let sent: CmdResult<(Exported, ExportTarget)> = async {
        Ok(match file_target {
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
        })
    }
    .await;
    // Yazılamadıysa kayıt silinir: satırlar gönderilmemiş kalır (Excel'de dosya yazılmadan önce
    // yedeklenir; Sheets'te yarıda yazılanlar kimlikleriyle bir daha yazılmaz).
    let (exported, target) = match sent {
        Ok(sent) => sent,
        Err(e) => {
            if let Err(clear) = save_interrupted(&app, None) {
                log_error!("aktarım kaydı silinemedi: {clear}");
            }
            return Err(e);
        }
    };
    // İşaretlenemezse kayıt kalır; bir sonraki açılışta dosyaya bakılarak çözülür.
    lock(&app.state::<Shared>().store)
        .mark_timesheet_exported(&ids, Utc::now(), &sheet.id, &sheet.consultant)
        .map_err(err)?;
    // Satırlar gönderildi; kayıt silinemezse bir sonraki çözümde zaten gönderilmiş görünürler.
    if let Err(e) = save_interrupted(&app, None) {
        log_error!("aktarım kaydı silinemedi: {e}");
    }
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

#[cfg(test)]
mod interrupted_tests {
    use super::*;
    use tracky_core::timesheet::EntryKind;

    fn entry(hour: u32, details: &str) -> TimesheetEntry {
        TimesheetEntry {
            date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            start: chrono::NaiveTime::from_hms_opt(hour, 0, 0).unwrap(),
            hours: 1.0,
            actual_hours: None,
            kind: EntryKind::Working,
            details: details.into(),
            party: "ADBA".into(),
            project_id: "togg".into(),
            division: "Togg".into(),
            coverage: Some(Vec::new()),
        }
    }

    #[test]
    fn only_rows_found_in_the_file_count_as_exported() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_tag(
                &Tag {
                    id: "togg".into(),
                    kind: TagKind::Project,
                    name: "Togg".into(),
                    color: 1,
                },
                0,
            )
            .unwrap();
        let a = store
            .save_timesheet_entry(None, &entry(9, "yazıldı"))
            .unwrap();
        let b = store
            .save_timesheet_entry(None, &entry(11, "yazılmadı"))
            .unwrap();
        let journal = Interrupted {
            sheet_id: "togg".into(),
            consultant: "Kaan".into(),
            excel: Some("/tmp/togg.xlsx".into()),
            ids: vec![a.clone(), b.clone(), "silinmis".into()],
            at: Utc::now(),
        };
        let pending = interrupted_pending(&store, &journal).unwrap();
        assert_eq!(pending.len(), 2, "bilinmeyen kimlik atlanır");
        assert_eq!(
            settle_interrupted(&store, &journal, &pending, &[true, false]).unwrap(),
            1
        );
        assert!(
            store
                .timesheet_entry(&a)
                .unwrap()
                .unwrap()
                .exported_at
                .is_some()
        );
        assert!(
            store
                .timesheet_entry(&b)
                .unwrap()
                .unwrap()
                .exported_at
                .is_none()
        );
        // Yeniden denenirse yalnızca dosyada olmayan kalır.
        let again = interrupted_pending(&store, &journal).unwrap();
        assert_eq!(
            again.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            [b.as_str()]
        );
    }
}
