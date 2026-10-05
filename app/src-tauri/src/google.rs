//! Google hesabıyla giriş: zaman çizelgesi tablosuna Sheets API ile doğrudan erişmek için
//! (Apps Script web uygulamasından çok daha hızlı; bkz. [`tracky_xlsx::gsheets`]).
//!
//! Kullanıcı kendi Google Cloud projesinde "Masaüstü uygulaması" OAuth istemcisi oluşturur ve
//! kimliğini Kum'a girer. Giriş tarayıcıda yapılır (PKCE'li yükleme akışı); Google onayı
//! bilgisayardaki geçici bir yerel adrese döndürür. Yenileme anahtarı yalnızca bu cihazda
//! saklanır (eşitlenmez); erişim anahtarı bellekte tutulur ve süresi dolmadan yenilenir.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tracky_core::Store;

use crate::lock;
use crate::tracking::Shared;

type CmdResult<T> = Result<T, String>;

/// Ayar anahtarı (eşitlenen ayarlar arasında değil: yenileme anahtarı cihazda kalır).
const KEY: &str = "google_oauth";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "openid email https://www.googleapis.com/auth/spreadsheets";
/// Tarayıcıda girişin beklendiği en uzun süre.
const LOGIN_WAIT: Duration = Duration::from_secs(300);
/// Erişim anahtarı bundan az ömrü kalınca yenilenir.
const REFRESH_MARGIN: Duration = Duration::from_secs(120);

/// Kayıtlı Google bağlantısı.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GoogleAuth {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: Option<String>,
    /// Bağlanan hesabın e-postası (gösterim için).
    pub email: Option<String>,
}

impl GoogleAuth {
    pub fn connected(&self) -> bool {
        self.refresh_token.is_some() && !self.client_id.is_empty()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleStatus {
    connected: bool,
    email: Option<String>,
    client_id: String,
    /// Gizli anahtar girilmiş (kendisi arayüze gönderilmez).
    has_secret: bool,
}

/// Bellekteki erişim anahtarı: (yenileme anahtarı, erişim anahtarı, geçerlilik sonu).
static ACCESS: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);
/// Süren girişi iptal eder.
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn load(store: &Store) -> GoogleAuth {
    store.setting(KEY).ok().flatten().unwrap_or_default()
}

fn status(auth: &GoogleAuth) -> GoogleStatus {
    GoogleStatus {
        connected: auth.connected(),
        email: auth.email.clone(),
        client_id: auth.client_id.clone(),
        has_secret: !auth.client_secret.is_empty(),
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

/// Anahtar ucuna form gönderir; yanıtın JSON'u ya da Google'ın hata açıklaması.
fn token_request(form: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let mut resp = agent()
        .post(TOKEN_URL)
        .send_form(form.iter().copied())
        .map_err(|e| format!("Google'a bağlanılamadı: {e}"))?;
    let status = resp.status().as_u16();
    let v: serde_json::Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("Google yanıtı okunamadı: {e}"))?;
    if status >= 400 {
        let code = v["error"].as_str().unwrap_or("hata");
        return Err(match code {
            "invalid_grant" => {
                "Google bağlantısının süresi doldu ya da kaldırıldı; Ayarlar → Zaman çizelgeleri'nden yeniden bağlan".into()
            }
            "invalid_client" => "OAuth istemci kimliği ya da gizli anahtarı yanlış".into(),
            _ => format!(
                "Google: {}",
                v["error_description"].as_str().unwrap_or(code)
            ),
        });
    }
    Ok(v)
}

/// Geçerli erişim anahtarı: bellekte süresi dolmamışsa o, değilse yenilenir (ağ; bloklar).
pub fn access_token(auth: &GoogleAuth) -> Result<String, String> {
    let refresh = auth.refresh_token.clone().ok_or("Google'a bağlı değil")?;
    if let Ok(cache) = ACCESS.lock()
        && let Some((r, token, until)) = cache.as_ref()
        && *r == refresh
        && Instant::now() + REFRESH_MARGIN < *until
    {
        return Ok(token.clone());
    }
    let v = token_request(&[
        ("client_id", &auth.client_id),
        ("client_secret", &auth.client_secret),
        ("refresh_token", &refresh),
        ("grant_type", "refresh_token"),
    ])?;
    let token = v["access_token"]
        .as_str()
        .ok_or("Google erişim anahtarı vermedi")?
        .to_string();
    let ttl = Duration::from_secs(v["expires_in"].as_u64().unwrap_or(3600));
    if let Ok(mut cache) = ACCESS.lock() {
        *cache = Some((refresh, token.clone(), Instant::now() + ttl));
    }
    Ok(token)
}

/// Erişim anahtarı reddedildiyse (401) bellekten atılır; sonraki istek yeniler.
pub fn forget_access() {
    if let Ok(mut cache) = ACCESS.lock() {
        *cache = None;
    }
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Kimlik belirtecinden (JWT) e-posta; imza doğrulanmaz, yalnızca gösterim içindir.
fn email_of(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v["email"].as_str().map(str::to_string)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        eprintln!("tarayıcı açılamadı: {e}");
    }
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Kum</title>\
<body style=\"font:15px -apple-system,system-ui,sans-serif;display:grid;place-items:center;height:90vh;color:#333\">\
<div><h2>Kum Google'a bağlandı</h2><p>Bu sekmeyi kapatıp Kum'a dönebilirsin.</p></div>";

/// Tarayıcıda giriş: yerel adreste Google'ın dönüşünü bekler, kodu anahtarlarla değiştirir.
fn login(
    client_id: &str,
    client_secret: &str,
) -> Result<(String, Option<String>, String, u64), String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("yerel adres açılamadı: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("yerel adres açılamadı: {e}"))?;
    let redirect = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().map_err(|e| e.to_string())?.port()
    );
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().simple().to_string();
    let url = tauri::Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ],
    )
    .map_err(|e| e.to_string())?;
    open_browser(url.as_str());

    let deadline = Instant::now() + LOGIN_WAIT;
    let code = loop {
        if CANCEL.swap(false, Ordering::AcqRel) {
            return Err("Google girişi iptal edildi.".into());
        }
        if Instant::now() > deadline {
            return Err("Google girişi zaman aşımına uğradı; tekrar dene.".into());
        }
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(format!("yerel adres hatası: {e}")),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        // "GET /?code=…&state=… HTTP/1.1"
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        let Ok(parsed) = tauri::Url::parse(&format!("http://127.0.0.1{path}")) else {
            continue;
        };
        let get = |k: &str| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.to_string())
        };
        if get("code").is_none() && get("error").is_none() {
            // Tarayıcının favicon gibi başka istekleri.
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let ok = get("error").is_none() && get("state").as_deref() == Some(state.as_str());
        let body = if ok {
            DONE_PAGE.to_string()
        } else {
            DONE_PAGE.replace("Kum Google'a bağlandı", "Giriş tamamlanmadı")
        };
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
        if let Some(e) = get("error") {
            return Err(if e == "access_denied" {
                "Google girişinde izin verilmedi.".into()
            } else {
                format!("Google girişi: {e}")
            });
        }
        if !ok {
            return Err("Google girişi doğrulanamadı; tekrar dene.".into());
        }
        break get("code").unwrap_or_default();
    };

    let v = token_request(&[
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect),
        ("grant_type", "authorization_code"),
    ])?;
    let refresh = v["refresh_token"]
        .as_str()
        .ok_or("Google yenileme anahtarı vermedi; tekrar dene")?
        .to_string();
    let access = v["access_token"].as_str().unwrap_or_default().to_string();
    let email = v["id_token"].as_str().and_then(email_of);
    Ok((
        refresh,
        email,
        access,
        v["expires_in"].as_u64().unwrap_or(3600),
    ))
}

#[tauri::command]
pub async fn google_status(app: AppHandle) -> CmdResult<GoogleStatus> {
    Ok(status(&load(&lock(&app.state::<Shared>().store))))
}

/// Tarayıcıda Google girişi yapar ve bağlantıyı kaydeder. Gizli anahtar boşsa kayıtlı olan.
#[tauri::command]
pub async fn google_connect(
    app: AppHandle,
    client_id: String,
    client_secret: String,
) -> CmdResult<GoogleStatus> {
    let saved = load(&lock(&app.state::<Shared>().store));
    let client_id = client_id.trim().to_string();
    let client_secret = match client_secret.trim() {
        "" => saved.client_secret.clone(),
        s => s.to_string(),
    };
    if !client_id.ends_with(".apps.googleusercontent.com") {
        return Err("İstemci kimliği …apps.googleusercontent.com ile bitmeli.".into());
    }
    if client_secret.is_empty() {
        return Err("İstemcinin gizli anahtarını (client secret) gir.".into());
    }
    CANCEL.store(false, Ordering::Release);
    let (id, secret) = (client_id.clone(), client_secret.clone());
    let (refresh, email, access, ttl) =
        tauri::async_runtime::spawn_blocking(move || login(&id, &secret))
            .await
            .map_err(|e| e.to_string())??;
    if let Ok(mut cache) = ACCESS.lock() {
        *cache = Some((
            refresh.clone(),
            access,
            Instant::now() + Duration::from_secs(ttl),
        ));
    }
    let auth = GoogleAuth {
        client_id,
        client_secret,
        refresh_token: Some(refresh),
        email,
    };
    lock(&app.state::<Shared>().store)
        .save_setting(KEY, &auth)
        .map_err(|e| e.to_string())?;
    Ok(status(&auth))
}

/// Süren girişi iptal eder.
#[tauri::command]
pub async fn google_cancel() -> CmdResult<()> {
    CANCEL.store(true, Ordering::Release);
    Ok(())
}

/// Bağlantıyı kaldırır (Google'daki izin de geri alınır); istemci kimliği kalır.
#[tauri::command]
pub async fn google_disconnect(app: AppHandle) -> CmdResult<GoogleStatus> {
    let mut auth = load(&lock(&app.state::<Shared>().store));
    if let Some(token) = auth.refresh_token.take() {
        // Olmasa da olur: kayıt silinir, Google hesabındaki izin elle de kaldırılabilir.
        let _ = tauri::async_runtime::spawn_blocking(move || {
            agent()
                .post("https://oauth2.googleapis.com/revoke")
                .send_form([("token", token.as_str())])
        })
        .await;
    }
    auth.email = None;
    forget_access();
    lock(&app.state::<Shared>().store)
        .save_setting(KEY, &auth)
        .map_err(|e| e.to_string())?;
    Ok(status(&auth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_read_from_the_id_token() {
        let payload = b64url(br#"{"email":"kaan@example.com","sub":"1"}"#);
        assert_eq!(
            email_of(&format!("e30.{payload}.imza")).as_deref(),
            Some("kaan@example.com")
        );
        assert_eq!(email_of("bozuk"), None);
        // PKCE: doğrulayıcının SHA-256'sı, dolgusuz base64url (Python hashlib ile karşılaştırıldı).
        assert_eq!(
            b64url(&Sha256::digest(
                b"dBjftJeZ4CVP-mJ92K27uhbUJU1p1r_wW1gFWFOEjXk"
            )),
            "ngF5GsXcbwljx6u133FFr3Xht9xooA_DuaX_3QwODtc"
        );
    }
}
