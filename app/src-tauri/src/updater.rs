//! Otomatik güncelleme: GitHub Releases'taki `latest.json`'u denetler, yeni
//! sürümü arka planda indirir (imzası eklenti tarafından doğrulanır). Kurulum ya elle
//! (kenar çubuğu, menü çubuğu) ya da kimse bilgisayar başında değilken kendiliğinden yapılır:
//! birden çok bilgisayar aynı sürümde kalsın. Kurulan sürümün notları ilk açılışta gösterilir.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{lock, tray};

/// Açılıştan sonra ilk denetim (takip ve senkronizasyon önce otursun).
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// Arka plan denetimlerinin aralığı: yeni sürüm birkaç dakika içinde görünsün (latest.json
/// küçük bir dosya). Duvar saatine göre ölçülür: Mac uykudan uyanınca geciken denetim bir
/// dakika içinde yapılır.
const INTERVAL: chrono::Duration = chrono::Duration::minutes(5);
const TICK: Duration = Duration::from_secs(60);
/// latest.json denetimi ve paket indirmesi için üst sınır: takılan bir istek `checking`
/// bayrağını sonsuza dek açık tutup sonraki tüm denetimleri durdurmasın.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// GitHub yayından hemen sonra paketi kısa süre 500 ile verebiliyor (0.9.83'te görüldü):
/// indirme bu kadar kez, her seferinde biraz daha bekleyerek denenir.
const DOWNLOAD_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Windows'ta kurulumdan hemen önce takip ve arka plan işleri durduruldu.
static STOPPED_FOR_INSTALL: AtomicBool = AtomicBool::new(false);
/// Hazır güncelleme bu kadar süre girdi olmadan geçince (video, görüşme yokken) kurulur.
const AUTO_INSTALL_IDLE_SECS: u64 = 10 * 60;
/// "Güncellemeleri kendiliğinden kur" (bu bilgisayarın ayarı; yoksa açık).
const AUTO_KEY: &str = "auto_update";
/// Kurulan sürümün notları; o sürüm ilk açıldığında gösterilir.
const WHATS_NEW_KEY: &str = "whats_new";
/// Sürümlerin dosyaları (`tauri.conf.json`'daki güncelleme adresiyle aynı depo): kurulumdan
/// önce notu kaydedilmemiş sürümün notu o sürümün `latest.json`'undan okunur.
const RELEASES: &str = "https://github.com/mylifer/tracky/releases/download";
/// Notu yazılmamış eski sürümlerin sabit metni: gösterilmez.
const PLACEHOLDER_NOTES: &str = "Güncellemeler uygulama içinden otomatik indirilir.";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current: String,
    /// Bulunan yeni sürüm.
    pub available: Option<String>,
    pub notes: Option<String>,
    /// İndirildi, kuruluma hazır.
    pub ready: bool,
    pub checking: bool,
    pub last_checked: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Hazır güncelleme boşta kendiliğinden kurulur.
    pub auto: bool,
}

/// Kurulan sürümün notları ("Yenilikler").
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct WhatsNew {
    pub version: String,
    pub notes: String,
    /// Kullanıcı kapattı.
    #[serde(default)]
    pub seen: bool,
}

#[derive(Default)]
pub struct UpdateState {
    status: Mutex<UpdateStatus>,
    /// İndirilmiş, imzası doğrulanmış paket.
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

fn auto_enabled(app: &AppHandle) -> bool {
    lock(&app.state::<crate::tracking::Shared>().store)
        .setting::<bool>(AUTO_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
}

pub fn start(app: &tauri::App) {
    app.manage(UpdateState {
        status: Mutex::new(UpdateStatus {
            current: app.package_info().version.to_string(),
            auto: auto_enabled(app.handle()),
            ..Default::default()
        }),
        pending: Mutex::new(None),
    });
    let handle = app.handle().clone();
    let _ = std::thread::Builder::new()
        .name("kum-updater".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                let last = lock(&handle.state::<UpdateState>().status).last_checked;
                if last.is_none_or(|t| Utc::now() - t >= INTERVAL) {
                    tauri::async_runtime::block_on(check_and_download(&handle));
                }
                install_when_away(&handle);
                std::thread::sleep(TICK);
            }
        });
}

fn set(app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
    let state = app.state::<UpdateState>();
    let snapshot = {
        let mut status = lock(&state.status);
        f(&mut status);
        status.clone()
    };
    let _ = app.emit("update", &snapshot);
    let (handle, s) = (app.clone(), snapshot.clone());
    let _ = app.run_on_main_thread(move || tray::set_update(&handle, &s));
    snapshot
}

/// Denetler; yeni sürüm varsa indirir ve kuruluma hazır tutar. Hazır bekleyen paketten
/// daha yeni bir sürüm çıktıysa onu indirip yerine koyar.
async fn check_and_download(app: &AppHandle) -> UpdateStatus {
    // Arka plan denetimi sürerken elle denetim (ya da tersi) aynı paketi ikinci kez
    // indirmesin: bayrak denetimle aynı kilit altında alınır.
    {
        let state = app.state::<UpdateState>();
        let mut status = lock(&state.status);
        if status.checking {
            return status.clone();
        }
        status.checking = true;
    }
    set(app, |s| s.error = None);
    let result = async {
        let handle = app.clone();
        let checked = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            // Windows'ta kurulum süreci uygulamayı kendisi kapatır, `RunEvent::Exit` gelmez:
            // takip süren oturumu önce burada yazsın. Eklentinin varsayılan temizliği de yapılır.
            .on_before_exit(move || {
                STOPPED_FOR_INSTALL.store(true, Ordering::Release);
                crate::shutdown_workers(&handle);
                handle.cleanup_before_exit();
            })
            .build()
            .map_err(|e| failed("denetlenemedi", e))?
            .check()
            .await;
        let mut update = match checked {
            Ok(Some(update)) => update,
            // Henüz hiç sürüm yayınlanmadıysa latest.json yoktur (404): yeni sürüm yok demektir.
            Ok(None) | Err(tauri_plugin_updater::Error::ReleaseNotFound) => {
                return Ok::<_, String>(None);
            }
            // Yayın sürerken latest.json'da bu sistemin kaydı henüz olmayabilir: diğer
            // sistemin paketi yüklenmiştir, bunun ki birkaç dakika sonra gelir.
            Err(
                e @ (tauri_plugin_updater::Error::TargetNotFound(_)
                | tauri_plugin_updater::Error::TargetsNotFound(_)),
            ) => {
                log_info!("güncelleme bu sistem için henüz yok: {e}");
                return Ok(None);
            }
            Err(e) => return Err(failed("denetlenemedi", e)),
        };
        let pending = lock(&app.state::<UpdateState>().pending)
            .as_ref()
            .map(|(u, _)| u.version.clone());
        // Hazır bekleyen paket yalnızca ondan daha yeni bir sürümle değiştirilir: yayından
        // kaldırılıp latest.json eski sürüme dönerse indirilmiş yeni paket korunur.
        if let Some(pending) = pending.as_deref()
            && !is_newer(&update.version, pending)
        {
            return Ok(None);
        }
        // Eklenti indirmeye denetimin zaman aşımını aktarmıyor.
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        // İndirme sürerken de arayüz ve menü çubuğu yeni sürümü göstersin. Hazır bekleyen
        // paket varsa o, yenisi inene kadar gösterilir ve kurulabilir kalır.
        if pending.is_none() {
            set(app, |s| {
                s.available = Some(update.version.clone());
                s.notes = update.body.clone();
                s.ready = false;
            });
        }
        let mut attempt = 1;
        let bytes = loop {
            match update.download(|_, _| {}, || {}).await {
                Ok(bytes) => break bytes,
                Err(
                    e @ (tauri_plugin_updater::Error::Reqwest(_)
                    | tauri_plugin_updater::Error::Network(_)),
                ) if attempt < DOWNLOAD_ATTEMPTS => {
                    log_info!(
                        "güncelleme indirilemedi ({attempt}. deneme), yeniden deneniyor: {e}"
                    );
                    tokio::time::sleep(RETRY_DELAY * attempt).await;
                    attempt += 1;
                }
                Err(e) => return Err(failed("indirilemedi", e)),
            }
        };
        Ok(Some((update, bytes)))
    }
    .await;

    match result {
        Ok(Some((update, bytes))) => {
            let (version, notes) = (update.version.clone(), update.body.clone());
            *lock(&app.state::<UpdateState>().pending) = Some((update, bytes));
            set(app, |s| {
                s.checking = false;
                s.last_checked = Some(Utc::now());
                s.available = Some(version);
                s.notes = notes;
                s.ready = true;
            })
        }
        Ok(None) => {
            let pending = lock(&app.state::<UpdateState>().pending).is_some();
            set(app, |s| {
                s.checking = false;
                s.last_checked = Some(Utc::now());
                if !pending {
                    s.available = None;
                    s.notes = None;
                }
            })
        }
        Err(e) => set(app, |s| {
            s.checking = false;
            s.last_checked = Some(Utc::now());
            s.error = Some(e);
        }),
    }
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    lock(&app.state::<UpdateState>().status).clone()
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> UpdateStatus {
    check_and_download(&app).await
}

/// İndirilmiş güncellemeyi kurar ve uygulamayı yeniden başlatır. `async`: kurulum ana
/// iş parçacığını tutmasın.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app)
}

pub(crate) fn install(app: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = lock(&app.state::<UpdateState>().pending).take() else {
        return Err("Kurulacak güncelleme yok".into());
    };
    // Yeni sürüm ilk açıldığında notları gösterilsin.
    save_whats_new(
        app,
        &WhatsNew {
            version: update.version.clone(),
            notes: update.body.clone().unwrap_or_default(),
            seen: false,
        },
    );
    if let Err(e) = update.install(bytes) {
        // İndirilen paket gitti: "hazır" görünmesin, bir sonraki denetim yeniden indirsin.
        let msg = failed("kurulamadı", e);
        set(app, |s| {
            s.ready = false;
            s.available = None;
            s.error = Some(msg.clone());
        });
        // Takip kurulumdan önce durdurulduysa yeniden başla; yoksa uygulama takipsiz kalırdı.
        if STOPPED_FOR_INSTALL.load(Ordering::Acquire) {
            app.request_restart();
        }
        return Err(msg);
    }
    // `restart()` ana iş parçacığından çağrılınca `RunEvent::Exit` atlanır: takip son
    // oturumu kaydetmeden kapanır, tek kopya eklentisi soketini temizlemez ve yeni süreç
    // kapanmakta olan eskisine devredip çıkabilir. `request_restart` olağan kapanıştan geçer.
    app.request_restart();
    Ok(())
}

/// Hazır güncellemeyi kimse bilgisayar başında değilken kurar: ayar açık, yeterince uzun süre
/// girdi yok (video ve görüşme girdi sayılır), zaman çizelgesi aktarılmıyor.
fn install_when_away(app: &AppHandle) {
    let state = app.state::<UpdateState>();
    let (ready, version) = {
        let status = lock(&state.status);
        (status.ready, status.available.clone().unwrap_or_default())
    };
    if !ready
        || crate::tracking::idle_seconds() < AUTO_INSTALL_IDLE_SECS
        || !auto_enabled(app)
        || crate::timesheet::export::not_exporting().is_err()
    {
        return;
    }
    log_info!("güncelleme boştayken kendiliğinden kuruluyor: {version}");
    if let Err(e) = install(app) {
        log_error!("güncelleme kendiliğinden kurulamadı: {e}");
    }
}

/// Bu sürümün notları: kurulumdan önce kaydedildiyse oradan, yoksa (elle kurulum, önceki
/// sürümden güncelleme) sürümün `latest.json`'undan bir kez okunur. Kapatıldıysa ya da not
/// yoksa `None`.
#[tauri::command]
pub async fn whats_new(app: AppHandle) -> Option<WhatsNew> {
    let current = app.package_info().version.to_string();
    let saved = saved_whats_new(&app).filter(|w| w.version == current);
    let entry = match saved {
        Some(w) => w,
        None => {
            let version = current.clone();
            let notes = tauri::async_runtime::spawn_blocking(move || release_notes(&version))
                .await
                .ok()
                .flatten()?;
            let w = WhatsNew {
                version: current,
                notes,
                seen: false,
            };
            save_whats_new(&app, &w);
            w
        }
    };
    (!entry.seen && !entry.notes.trim().is_empty() && entry.notes.trim() != PLACEHOLDER_NOTES)
        .then_some(entry)
}

#[tauri::command]
pub fn dismiss_whats_new(app: AppHandle) {
    if let Some(mut w) = saved_whats_new(&app) {
        w.seen = true;
        save_whats_new(&app, &w);
    }
}

fn saved_whats_new(app: &AppHandle) -> Option<WhatsNew> {
    lock(&app.state::<crate::tracking::Shared>().store)
        .setting::<WhatsNew>(WHATS_NEW_KEY)
        .ok()
        .flatten()
}

fn save_whats_new(app: &AppHandle, w: &WhatsNew) {
    if let Err(e) =
        lock(&app.state::<crate::tracking::Shared>().store).save_setting(WHATS_NEW_KEY, w)
    {
        log_error!("sürüm notları kaydedilemedi: {e}");
    }
}

/// Sürümün notları (o sürümün yayınındaki `latest.json`); okunamazsa `None`.
fn release_notes(version: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(CHECK_TIMEOUT))
        .build()
        .into();
    let json: serde_json::Value = agent
        .get(&format!("{RELEASES}/v{version}/latest.json"))
        .call()
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;
    Some(json.get("notes")?.as_str()?.to_string())
}

/// "Güncellemeleri kendiliğinden kur" ayarı.
#[tauri::command]
pub fn set_auto_update(app: AppHandle, enabled: bool) -> Result<UpdateStatus, String> {
    lock(&app.state::<crate::tracking::Shared>().store)
        .save_setting(AUTO_KEY, &enabled)
        .map_err(|e| e.to_string())?;
    Ok(set(&app, |s| s.auto = enabled))
}

/// `a`, `b`'den yeni mi (semver; çözülemezse metin farkı yeni sayılır).
fn is_newer(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a != b,
    }
}

/// Hatanın aslını günlüğe yazar (arayüzdeki metin ayrıntıyı gizler), gösterilecek metni döndürür.
fn failed(what: &str, e: tauri_plugin_updater::Error) -> String {
    log_error!("güncelleme {what}: {e}");
    message(e)
}

/// Güncelleyici hatasını kullanıcıya gösterilecek Türkçe metne çevirir.
fn message(e: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Reqwest(_) => {
            "Güncelleme sunucusuna ulaşılamadı. İnternet bağlantını denetleyip tekrar dene.".into()
        }
        // Sunucu yanıt verdi ama paketi vermedi (ör. GitHub'ın geçici 500 hatası).
        E::Network(_) => {
            "Güncelleme sunucusu paketi şu an vermiyor; birkaç dakika sonra kendiliğinden yeniden denenir.".into()
        }
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => {
            "İndirilen güncellemenin imzası doğrulanamadı; kurulmadı.".into()
        }
        E::TargetNotFound(_) | E::TargetsNotFound(_) => {
            "Yeni sürümde bu sistem için paket yok.".into()
        }
        other => format!("Güncelleme denetlenemedi: {other}"),
    }
}
