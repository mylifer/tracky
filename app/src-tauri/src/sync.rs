//! Supabase senkronizasyonu: ayarlar, oturum ve arka plan döngüsü.
//!
//! Bağlantı ayarları ve oturum jetonları yalnızca bu cihazın ayarlarında
//! tutulur (settings tablosu senkronize edilmez).

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracky_core::sync::SyncSummary;
use tracky_sync::{AuthSession, Client, Config};

use crate::lock;
use crate::tracking::Shared;

const CONFIG_KEY: &str = "sync_config";
const AUTH_KEY: &str = "sync_auth";
/// Yerel eşitleme durumunun ait olduğu "proje|kullanıcı".
const OWNER_KEY: &str = "sync_owner";
/// Arka planda bu aralıkla eşitlenir.
const INTERVAL: Duration = Duration::from_secs(5 * 60);

pub enum SyncCommand {
    Now,
    Shutdown,
}

pub struct SyncWorker {
    pub tx: Mutex<Sender<SyncCommand>>,
    pub last: Mutex<Option<LastSync>>,
    /// Bir eşitleme sürerken tutulur: hesap değişince imleç sıfırlama, süren
    /// eşitlemenin (eski hesabın) imleçleri yazmasını bekler.
    pub gate: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    pub at: DateTime<Utc>,
    pub ok: bool,
    pub message: String,
    pub summary: Option<SyncSummary>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    configured: bool,
    url: Option<String>,
    email: Option<String>,
    last: Option<LastSync>,
}

type CmdResult<T> = Result<T, String>;

fn load<T: serde::de::DeserializeOwned>(app: &AppHandle, key: &str) -> Option<T> {
    lock(&app.state::<Shared>().store)
        .setting(key)
        .ok()
        .flatten()
}

fn save<T: Serialize>(app: &AppHandle, key: &str, value: &T) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .save_setting(key, value)
        .map_err(|e| e.to_string())
}

/// Ayar kaldırma: JSON `null` kaydedilir, okumada `None` gibi davranır.
fn clear(app: &AppHandle, key: &str) -> CmdResult<()> {
    save(app, key, &serde_json::Value::Null)
}

fn status(app: &AppHandle) -> SyncStatus {
    let config: Option<Config> = load(app, CONFIG_KEY);
    let auth: Option<AuthSession> = load(app, AUTH_KEY);
    SyncStatus {
        configured: config.is_some(),
        url: config.map(|c| c.url),
        email: auth.map(|a| a.email),
        last: lock(&app.state::<SyncWorker>().last).clone(),
    }
}

/// Bir kez eşitler; gerekirse oturumu yeniler. Giriş yoksa `Ok(None)`.
fn sync_once(app: &AppHandle) -> Result<Option<SyncSummary>, String> {
    let (Some(config), Some(mut auth)) = (
        load::<Config>(app, CONFIG_KEY),
        load::<AuthSession>(app, AUTH_KEY),
    ) else {
        return Ok(None);
    };
    let client = Client::new(config);
    if auth.needs_refresh() {
        auth = refresh(app, &client, &auth)?;
    }
    let store = &app.state::<Shared>().store;
    let attempt = |auth: &AuthSession| {
        tracky_core::sync::run(store, &mut client.remote(auth), &auth.user_id)
            .map_err(|e| e.to_string())
    };
    let result = match attempt(&auth) {
        // Jeton beklenenden önce geçersiz kaldıysa bir kez yenileyip tekrar dene.
        Err(e) if e.contains("[401]") => attempt(&refresh(app, &client, &auth)?),
        other => other,
    };
    result.map(Some)
}

/// Jetonu yeniler ve kaydeder. Bu arada kullanıcı çıkış yaptıysa ya da başka
/// hesapla girdiyse yazmaz (yoksa çıkış yapan kullanıcı sessizce geri girerdi).
fn refresh(app: &AppHandle, client: &Client, auth: &AuthSession) -> Result<AuthSession, String> {
    let fresh = client.refresh(auth).map_err(|e| e.to_string())?;
    let stored: Option<AuthSession> = load(app, AUTH_KEY);
    if stored.map(|s| s.refresh_token) != Some(auth.refresh_token.clone()) {
        return Err("Oturum değişti; eşitleme atlandı".into());
    }
    save(app, AUTH_KEY, &fresh)?;
    Ok(fresh)
}

/// Sunucu şeması eskiyse (müşteriler tablosu ya da sütunu yok) ne yapılacağını söyler.
fn migration_hint(message: String) -> String {
    let lower = message.to_lowercase();
    if lower.contains("client")
        && (lower.contains("does not exist") || lower.contains("could not find"))
    {
        format!(
            "{message} — Supabase SQL Editor'da supabase/migrations/0005_clients.sql dosyasını bir kez çalıştır"
        )
    } else {
        message
    }
}

fn record(app: &AppHandle, result: Result<Option<SyncSummary>, String>) {
    let last = match result {
        Ok(None) => return,
        Ok(Some(summary)) => LastSync {
            at: Utc::now(),
            ok: true,
            message: if summary.skipped > 0 {
                format!(
                    "{} gönderildi, {} alındı; {} kayıt bu sürümde okunamadığı için atlandı (Kum'u güncelle)",
                    summary.pushed, summary.pulled, summary.skipped
                )
            } else {
                format!("{} gönderildi, {} alındı", summary.pushed, summary.pulled)
            },
            summary: Some(summary),
        },
        Err(message) => LastSync {
            at: Utc::now(),
            ok: false,
            message: migration_hint(message),
            summary: None,
        },
    };
    *lock(&app.state::<SyncWorker>().last) = Some(last);
    let _ = app.emit("sync", status(app));
}

/// Arka plan döngüsü: aralıkla ya da istekle eşitler.
pub fn run(app: AppHandle, rx: Receiver<SyncCommand>) {
    // Açılışta takip kendini toparlasın diye kısa bir gecikme.
    let mut wait = Duration::from_secs(20);
    loop {
        match rx.recv_timeout(wait) {
            Ok(SyncCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(SyncCommand::Now) | Err(RecvTimeoutError::Timeout) => {}
        }
        {
            let worker = app.state::<SyncWorker>();
            let _gate = lock(&worker.gate);
            record(&app, sync_once(&app));
        }
        wait = INTERVAL;
    }
    // Kapanışta beklemeyiz (ağ yavaşsa uygulama kapanmaz gibi görünür);
    // kalan değişiklikler bir sonraki açılışta gönderilir.
}

#[tauri::command]
pub async fn sync_status(app: AppHandle) -> SyncStatus {
    status(&app)
}

#[tauri::command]
pub async fn sync_configure(
    app: AppHandle,
    url: String,
    anon_key: String,
) -> CmdResult<SyncStatus> {
    let url = url.trim().trim_end_matches('/').to_string();
    if !url.starts_with("https://") {
        return Err("Adres https:// ile başlamalı (örn. https://abcd.supabase.co)".into());
    }
    if anon_key.trim().is_empty() {
        return Err("Anahtar boş olamaz".into());
    }
    save(
        &app,
        CONFIG_KEY,
        &Config {
            url,
            anon_key: anon_key.trim().to_string(),
        },
    )?;
    clear(&app, AUTH_KEY)?;
    Ok(status(&app))
}

/// Ağ çağrısı yaptığı için arayüzü kilitlememek adına ayrı iş parçacığında çalışır.
#[tauri::command]
pub async fn sync_sign_in(
    app: AppHandle,
    email: String,
    password: String,
    sign_up: bool,
) -> CmdResult<SyncStatus> {
    let config: Config = load(&app, CONFIG_KEY).ok_or("Önce Supabase bağlantısını kaydet")?;
    let url = config.url.clone();
    let auth = tauri::async_runtime::spawn_blocking(move || {
        let client = Client::new(config);
        let email = email.trim();
        if sign_up {
            client.sign_up(email, &password)
        } else {
            client.sign_in(email, &password)
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    // Başka hesap ya da proje: eski imleçler ve "gönderildi" işaretleri bu hesap için
    // anlamsız; her şeyi yeniden gönder ve baştan çek.
    let owner = format!("{url}|{}", auth.user_id);
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<()> {
        let app = handle;
        let worker = app.state::<SyncWorker>();
        let _gate = lock(&worker.gate);
        if load::<String>(&app, OWNER_KEY).as_deref() != Some(owner.as_str()) {
            lock(&app.state::<Shared>().store)
                .reset_sync_state()
                .map_err(|e| e.to_string())?;
            save(&app, OWNER_KEY, &owner)?;
        }
        save(&app, AUTH_KEY, &auth)
    })
    .await
    .map_err(|e| e.to_string())??;
    sync_now(app.clone());
    Ok(status(&app))
}

#[tauri::command]
pub async fn sync_sign_out(app: AppHandle) -> CmdResult<SyncStatus> {
    clear(&app, AUTH_KEY)?;
    *lock(&app.state::<SyncWorker>().last) = None;
    Ok(status(&app))
}

/// Bağlantıyı tamamen kaldırır (yerel veriler kalır).
#[tauri::command]
pub async fn sync_disconnect(app: AppHandle) -> CmdResult<SyncStatus> {
    clear(&app, AUTH_KEY)?;
    clear(&app, CONFIG_KEY)?;
    *lock(&app.state::<SyncWorker>().last) = None;
    Ok(status(&app))
}

#[tauri::command]
pub fn sync_now(app: AppHandle) {
    let _ = lock(&app.state::<SyncWorker>().tx).send(SyncCommand::Now);
}

/// Kapanışta döngüyü durdurur.
pub fn shutdown(app: &AppHandle) {
    let _ = lock(&app.state::<SyncWorker>().tx).send(SyncCommand::Shutdown);
}

/// Senkronizasyon döngüsünü başlatır.
pub fn start(app: &tauri::App) -> std::io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.manage(SyncWorker {
        tx: Mutex::new(tx),
        last: Mutex::new(None),
        gate: Mutex::new(()),
    });
    let handle = app.handle().clone();
    std::thread::Builder::new()
        .name("kum-sync".into())
        .spawn(move || run(handle, rx))
        .map(drop)
}
