//! Supabase istemcisi: e-posta/şifre ile oturum ve PostgREST üzerinden senkronizasyon.
//!
//! Sunucu şeması `supabase/migrations/` altındadır (RLS: herkes yalnız kendi satırlarını görür).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracky_core::sync::Remote;

const TIMEOUT: Duration = Duration::from_secs(30);
/// Erişim jetonu bitmeden bu kadar önce yenilenir.
const REFRESH_MARGIN_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Proje adresi, örn. `https://abcd.supabase.co`
    pub url: String,
    /// Projenin herkese açık (anon/publishable) anahtarı.
    pub anon_key: String,
}

impl Config {
    fn base(&self) -> String {
        self.url.trim().trim_end_matches('/').to_string()
    }
}

/// Giriş yapılmış oturum; yerel ayarlarda saklanır.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix saniye.
    pub expires_at: u64,
    pub user_id: String,
    pub email: String,
}

impl AuthSession {
    pub fn needs_refresh(&self) -> bool {
        now_secs() + REFRESH_MARGIN_SECS >= self.expires_at
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bağlantı hatası: {0}")]
    Http(#[from] ureq::Error),
    #[error("{0}")]
    Api(String),
    #[error("Kayıt alındı. E-postana gelen bağlantıyla hesabını doğrula, sonra giriş yap.")]
    ConfirmEmail,
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Client {
    config: Config,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(config: Config) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        Self { config, agent }
    }

    pub fn sign_in(&self, email: &str, password: &str) -> Result<AuthSession> {
        let body = self.auth_post(
            "token?grant_type=password",
            json!({ "email": email, "password": password }),
        )?;
        session_from(&body)
    }

    /// E-posta doğrulaması açıksa oturum dönmez; `Error::ConfirmEmail`.
    pub fn sign_up(&self, email: &str, password: &str) -> Result<AuthSession> {
        let body = self.auth_post("signup", json!({ "email": email, "password": password }))?;
        if body.get("access_token").is_none() {
            return Err(Error::ConfirmEmail);
        }
        session_from(&body)
    }

    pub fn refresh(&self, session: &AuthSession) -> Result<AuthSession> {
        let body = self.auth_post(
            "token?grant_type=refresh_token",
            json!({ "refresh_token": session.refresh_token }),
        )?;
        session_from(&body)
    }

    fn auth_post(&self, path: &str, body: Value) -> Result<Value> {
        let mut resp = self
            .agent
            .post(format!("{}/auth/v1/{path}", self.config.base()))
            .header("apikey", &self.config.anon_key)
            .send_json(body)?;
        let status = resp.status().as_u16();
        let body: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if status >= 400 {
            return Err(Error::Api(api_message(&body, status)));
        }
        Ok(body)
    }

    /// Bu oturumla senkronizasyon için uzak depo.
    pub fn remote<'a>(&'a self, session: &'a AuthSession) -> SupabaseRemote<'a> {
        SupabaseRemote {
            client: self,
            session,
        }
    }
}

pub struct SupabaseRemote<'a> {
    client: &'a Client,
    session: &'a AuthSession,
}

impl SupabaseRemote<'_> {
    fn rest(&self, table: &str) -> String {
        format!("{}/rest/v1/{table}", self.client.config.base())
    }

    fn check(
        &self,
        resp: &mut ureq::http::Response<ureq::Body>,
    ) -> std::result::Result<(), String> {
        let status = resp.status().as_u16();
        if status < 400 {
            return Ok(());
        }
        let body: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        Err(api_message(&body, status))
    }
}

impl Remote for SupabaseRemote<'_> {
    fn push(&mut self, table: &str, rows: &[Value]) -> std::result::Result<(), String> {
        let mut resp = self
            .client
            .agent
            .post(self.rest(table))
            .query("on_conflict", "user_id,id")
            .header("apikey", &self.client.config.anon_key)
            .header(
                "Authorization",
                format!("Bearer {}", self.session.access_token),
            )
            .header("Prefer", "resolution=merge-duplicates,return=minimal")
            .send_json(rows)
            .map_err(|e| e.to_string())?;
        self.check(&mut resp)
    }

    fn pull(
        &mut self,
        table: &str,
        since: Option<&str>,
        skip_writer: Option<&str>,
        limit: usize,
    ) -> std::result::Result<Vec<Value>, String> {
        let mut req = self
            .client
            .agent
            .get(self.rest(table))
            .query("select", "*")
            .query("order", "server_updated_at.asc")
            .query("limit", limit.to_string())
            .header("apikey", &self.client.config.anon_key)
            .header(
                "Authorization",
                format!("Bearer {}", self.session.access_token),
            );
        if let Some(since) = since {
            req = req.query("server_updated_at", format!("gt.{}", utc_z(since)));
        }
        if let Some(writer) = skip_writer {
            // `neq` NULL'ları da dışlar; 0003 öncesi yazılan satırlar da gelmeli.
            req = req.query("or", format!("(writer.is.null,writer.neq.{writer})"));
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        self.check(&mut resp)?;
        resp.body_mut()
            .read_json::<Vec<Value>>()
            .map_err(|e| e.to_string())
    }
}

fn session_from(body: &Value) -> Result<AuthSession> {
    let s = |v: &Value| v.as_str().map(str::to_string);
    let missing = || Error::Api("sunucu yanıtında oturum bilgisi eksik".into());
    // Süre yerel saatle hesaplanır; sunucunun mutlak `expires_at` değeri yerel
    // saat geri kaldığında süresi dolmuş jetonun gönderilmesine yol açar.
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    Ok(AuthSession {
        access_token: s(&body["access_token"]).ok_or_else(missing)?,
        refresh_token: s(&body["refresh_token"]).ok_or_else(missing)?,
        expires_at: now_secs() + expires_in,
        user_id: s(&body["user"]["id"]).ok_or_else(missing)?,
        email: s(&body["user"]["email"]).unwrap_or_default(),
    })
}

/// GoTrue ve PostgREST hata gövdelerinden okunabilir mesaj.
fn api_message(body: &Value, status: u16) -> String {
    let text = ["msg", "message", "error_description", "error"]
        .iter()
        .find_map(|k| body[*k].as_str())
        .unwrap_or("bilinmeyen hata");
    let hint = match status {
        400 if text.contains("Invalid login") => " (e-posta ya da şifre hatalı)",
        401 => " (oturum geçersiz; yeniden giriş yap)",
        404 => " (tablo bulunamadı; Supabase şemasını kurdun mu?)",
        _ => "",
    };
    format!("{text}{hint} [{status}]")
}

/// "+00:00" yerine "Z": adreste "+" boşluk olarak okunabilir.
fn utc_z(ts: &str) -> String {
    match ts.strip_suffix("+00:00") {
        Some(rest) => format!("{rest}Z"),
        None => ts.to_string(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    /// Tek istek alıp verilen yanıtı dönen yerel sunucu; isteği geri bildirir.
    fn serve_once(
        status: u16,
        body: &'static str,
    ) -> (String, mpsc::Receiver<(String, String, String)>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", server.server_addr().to_ip().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut req = server.recv().unwrap();
            let mut content = String::new();
            req.as_reader().read_to_string(&mut content).unwrap();
            let headers = req
                .headers()
                .iter()
                .map(|h| format!("{}: {}", h.field, h.value).to_lowercase())
                .collect::<Vec<_>>()
                .join("\n");
            tx.send((req.url().to_string(), headers, content)).unwrap();
            let resp = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(
                    "Content-Type: application/json"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                );
            req.respond(resp).unwrap();
        });
        (addr, rx)
    }

    fn client(url: String) -> Client {
        Client::new(Config {
            url: format!("{url}/"),
            anon_key: "anon".into(),
        })
    }

    #[test]
    fn signs_in_and_parses_session() {
        let (url, rx) = serve_once(
            200,
            r#"{"access_token":"a","refresh_token":"r","expires_in":3600,"user":{"id":"u1","email":"k@x.com"}}"#,
        );
        let s = client(url).sign_in("k@x.com", "pw").unwrap();
        assert_eq!((s.user_id.as_str(), s.email.as_str()), ("u1", "k@x.com"));
        assert!(!s.needs_refresh());
        let (path, headers, body) = rx.recv().unwrap();
        assert_eq!(path, "/auth/v1/token?grant_type=password");
        assert!(headers.contains("apikey: anon"));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["password"], "pw");
    }

    #[test]
    fn reports_api_errors_and_email_confirmation() {
        let (url, _rx) = serve_once(400, r#"{"error_description":"Invalid login credentials"}"#);
        let err = client(url).sign_in("k@x.com", "bad").unwrap_err();
        assert!(err.to_string().contains("şifre hatalı"), "{err}");

        let (url, _rx) = serve_once(200, r#"{"id":"u1","email":"k@x.com"}"#);
        assert!(matches!(
            client(url).sign_up("k@x.com", "pw"),
            Err(Error::ConfirmEmail)
        ));
    }

    #[test]
    fn pull_builds_postgrest_query() {
        let (url, rx) = serve_once(200, r#"[{"id":"1"}]"#);
        let c = client(url);
        let session = AuthSession {
            access_token: "tok".into(),
            refresh_token: "r".into(),
            expires_at: u64::MAX,
            user_id: "u1".into(),
            email: String::new(),
        };
        let rows = c
            .remote(&session)
            .pull(
                "sessions",
                Some("2026-10-02T12:00:00.123+00:00"),
                Some("d1"),
                1000,
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
        let (path, headers, _) = rx.recv().unwrap();
        assert!(path.starts_with("/rest/v1/sessions?"), "{path}");
        assert!(path.contains("order=server_updated_at.asc"));
        assert!(
            path.contains("server_updated_at=gt.2026-10-02T12%3A00%3A00.123Z")
                || path.contains("server_updated_at=gt.2026-10-02T12:00:00.123Z"),
            "{path}"
        );
        assert!(
            path.contains("writer.is.null") && path.contains("writer.neq.d1"),
            "{path}"
        );
        assert!(headers.contains("authorization: bearer tok"));
    }

    #[test]
    fn push_upserts_on_user_and_id() {
        let (url, rx) = serve_once(201, "");
        let c = client(url);
        let session = AuthSession {
            access_token: "tok".into(),
            refresh_token: "r".into(),
            expires_at: u64::MAX,
            user_id: "u1".into(),
            email: String::new(),
        };
        c.remote(&session)
            .push("tags", &[json!({"id": "t1"})])
            .unwrap();
        let (path, headers, body) = rx.recv().unwrap();
        assert!(
            path.contains("on_conflict=user_id%2Cid") || path.contains("on_conflict=user_id,id"),
            "{path}"
        );
        assert!(headers.contains("resolution=merge-duplicates"));
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!([{"id": "t1"}])
        );
    }
}
