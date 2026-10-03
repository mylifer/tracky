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
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

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
                tauri::async_runtime::block_on(check_and_download(&handle));
                std::thread::sleep(INTERVAL);
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

/// Denetler; yeni sürüm varsa indirir ve kuruluma hazır tutar.
async fn check_and_download(app: &AppHandle) -> UpdateStatus {
    if lock(&app.state::<UpdateState>().pending).is_some() {
        return lock(&app.state::<UpdateState>().status).clone();
    }
    set(app, |s| {
        s.checking = true;
        s.error = None;
    });
    let result = async {
        let Some(update) = app
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok::<_, String>(None);
        };
        let bytes = update
            .download(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
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
        Ok(None) => set(app, |s| {
            s.checking = false;
            s.last_checked = Some(Utc::now());
        }),
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

/// İndirilmiş güncellemeyi kurar ve uygulamayı yeniden başlatır.
#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app)
}

pub(crate) fn install(app: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = lock(&app.state::<UpdateState>().pending).take() else {
        return Err("Kurulacak güncelleme yok".into());
    };
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}
