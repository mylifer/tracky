//! Otomatik güncelleme: GitHub Releases'taki `latest.json`'u denetler, yeni
//! sürümü arka planda indirir (imzası eklenti tarafından doğrulanır), kurulumu
//! kullanıcı onayıyla yapar (Windows'ta kurulum uygulamayı kapattığı için).

use std::sync::Mutex;
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
}

#[derive(Default)]
pub struct UpdateState {
    status: Mutex<UpdateStatus>,
    /// İndirilmiş, imzası doğrulanmış paket.
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

pub fn start(app: &tauri::App) {
    app.manage(UpdateState {
        status: Mutex::new(UpdateStatus {
            current: app.package_info().version.to_string(),
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
        let checked = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .build()
            .map_err(message)?
            .check()
            .await;
        let mut update = match checked {
            Ok(Some(update)) => update,
            // Henüz hiç sürüm yayınlanmadıysa latest.json yoktur (404): yeni sürüm yok demektir.
            Ok(None) | Err(tauri_plugin_updater::Error::ReleaseNotFound) => {
                return Ok::<_, String>(None);
            }
            Err(e) => return Err(message(e)),
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
        let bytes = update.download(|_, _| {}, || {}).await.map_err(message)?;
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
    if let Err(e) = update.install(bytes) {
        // İndirilen paket gitti: "hazır" görünmesin, bir sonraki denetim yeniden indirsin.
        let msg = message(e);
        set(app, |s| {
            s.ready = false;
            s.available = None;
            s.error = Some(msg.clone());
        });
        return Err(msg);
    }
    // `restart()` ana iş parçacığından çağrılınca `RunEvent::Exit` atlanır: takip son
    // oturumu kaydetmeden kapanır, tek kopya eklentisi soketini temizlemez ve yeni süreç
    // kapanmakta olan eskisine devredip çıkabilir. `request_restart` olağan kapanıştan geçer.
    app.request_restart();
    Ok(())
}

/// `a`, `b`'den yeni mi (semver; çözülemezse metin farkı yeni sayılır).
fn is_newer(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a != b,
    }
}

/// Güncelleyici hatasını kullanıcıya gösterilecek Türkçe metne çevirir.
fn message(e: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Reqwest(_) | E::Network(_) => {
            "Güncelleme sunucusuna ulaşılamadı. İnternet bağlantını denetleyip tekrar dene.".into()
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
