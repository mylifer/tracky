//! Supabase Realtime: hesabın dürtmelerini dinler (`sync_nudges`, supabase/migrations/0017).
//! Bir bilgisayar düzenlemesini gönderince dürter; öteki bilgisayarlar burada duyar ve hemen
//! eşitler (yoksa sıradaki 5 dakikalık eşitlemeyi beklerdi). Yalnızca "bir şey değişti" sinyali
//! taşınır; veriler her zamanki gibi eşitlemeyle gelir.
//!
//! Protokol Phoenix kanallarıdır (JSON, `vsn=1.0.0`): kanala katılma, 25 saniyede bir kalp
//! atışı, jeton yenilenince `access_token`, satır değişiminde `postgres_changes`.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::Config;

/// Dürtmelerin tablosu.
pub const NUDGES: &str = "sync_nudges";
const TOPIC: &str = "realtime:kum-nudges";
/// Sunucu bundan uzun sessiz kalan bağlantıyı kapatır (Phoenix varsayılanı 60 sn).
const HEARTBEAT: Duration = Duration::from_secs(25);
/// Okuma bu kadar bekler; arada durdurma, kalp atışı ve jeton denetlenir.
const POLL: Duration = Duration::from_millis(500);

/// Kanaldan gelen, işe yarayan olaylar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Kanala katılındı.
    Joined,
    /// Bir açılış dürttü; yazanın kimliği.
    Nudge(String),
    /// Kanal reddedildi ya da kapandı.
    Failed(String),
    Other,
}

/// Dinlemenin bağlamı.
pub struct Listen<'a> {
    pub config: &'a Config,
    pub user_id: &'a str,
    /// Güncel erişim jetonu (yenilenince değişir); `None`: oturum kapandı, dinleme biter.
    pub token: &'a mut dyn FnMut() -> Option<String>,
    /// Durdurulsun mu (kapanış, hesap değişti).
    pub stop: &'a dyn Fn() -> bool,
    /// Başka bir açılış dürttü (yazanın kimliği).
    pub on_nudge: &'a mut dyn FnMut(&str),
}

/// Projenin Realtime adresi: `https://x.supabase.co` → `wss://x.supabase.co/realtime/v1/websocket`.
pub fn socket_url(config: &Config) -> String {
    let base = config.base();
    let base = match base.strip_prefix("https://") {
        Some(rest) => format!("wss://{rest}"),
        None => base.replacen("http://", "ws://", 1),
    };
    format!(
        "{base}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
        config.anon_key
    )
}

/// Kanala katılma: yalnızca bu kullanıcının dürtme satırının değişimleri (satır güvenliği de
/// başkasınınkini göstermez).
pub fn join_message(config: &Config, user_id: &str, token: &str) -> String {
    json!({
        "topic": TOPIC,
        "event": "phx_join",
        "payload": {
            "config": {
                "broadcast": { "ack": false, "self": false },
                "presence": { "key": "" },
                "postgres_changes": [{
                    "event": "*",
                    "schema": config.profile().unwrap_or("public"),
                    "table": NUDGES,
                    "filter": format!("user_id=eq.{user_id}"),
                }],
                "private": false,
            },
            "access_token": token,
        },
        "ref": "1",
        "join_ref": "1",
    })
    .to_string()
}

fn message(topic: &str, event: &str, payload: Value, n: u64) -> String {
    json!({ "topic": topic, "event": event, "payload": payload, "ref": n.to_string() }).to_string()
}

/// Sunucudan gelen metni yorumlar.
pub fn parse(text: &str) -> Event {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Event::Other;
    };
    let payload = &v["payload"];
    match v["event"].as_str().unwrap_or_default() {
        "phx_reply" if v["ref"] == "1" => match payload["status"].as_str() {
            Some("ok") => Event::Joined,
            _ => Event::Failed(reason(payload)),
        },
        "system" if payload["status"] == "error" => Event::Failed(reason(payload)),
        "phx_error" | "phx_close" => Event::Failed(format!("kanal kapandı ({})", v["event"])),
        "postgres_changes" => {
            let record = &payload["data"]["record"];
            match record["writer"].as_str() {
                Some(writer) => Event::Nudge(writer.to_string()),
                None => Event::Other,
            }
        }
        _ => Event::Other,
    }
}

fn reason(payload: &Value) -> String {
    let r = &payload["response"]["reason"];
    let text = r
        .as_str()
        .or_else(|| payload["message"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| payload.to_string());
    format!("Realtime: {text}")
}

/// Okuma bekleme süresi dolunca dönsün: arada durdurma ve kalp atışı denetlenir.
fn set_poll(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let stream = match socket.get_mut() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_mut(),
        _ => return,
    };
    let _ = stream.set_read_timeout(Some(POLL));
}

/// Bağlanır ve durdurulana ya da oturum kapanana kadar dinler (`Ok`). Bağlantı koparsa ya
/// da kanal reddedilirse hatayla döner; yeniden bağlanmak çağıranın işidir.
pub fn listen(l: Listen) -> Result<(), String> {
    let Some(mut token) = (l.token)() else {
        return Ok(());
    };
    let (mut socket, _) =
        tungstenite::connect(socket_url(l.config)).map_err(|e| format!("Realtime: {e}"))?;
    set_poll(&mut socket);
    let send = |socket: &mut WebSocket<_>, text: String| {
        socket
            .send(Message::text(text))
            .map_err(|e| format!("Realtime: {e}"))
    };
    send(&mut socket, join_message(l.config, l.user_id, &token))?;
    let mut n = 1;
    let mut beat = Instant::now();
    loop {
        if (l.stop)() {
            let _ = socket.close(None);
            return Ok(());
        }
        if beat.elapsed() >= HEARTBEAT {
            beat = Instant::now();
            n += 1;
            send(&mut socket, message("phoenix", "heartbeat", json!({}), n))?;
            // Jeton yenilendiyse kanal yeni jetonla sürer (eskisi bitince kanal kapanırdı).
            match (l.token)() {
                None => {
                    let _ = socket.close(None);
                    return Ok(());
                }
                Some(fresh) if fresh != token => {
                    token = fresh;
                    n += 1;
                    let payload = json!({ "access_token": token });
                    send(&mut socket, message(TOPIC, "access_token", payload, n))?;
                }
                Some(_) => {}
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => match parse(&text) {
                Event::Nudge(writer) => (l.on_nudge)(&writer),
                Event::Failed(why) => return Err(why),
                Event::Joined | Event::Other => {}
            },
            Ok(Message::Close(_)) => return Err("Realtime: sunucu bağlantıyı kapattı".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(format!("Realtime: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            url: "https://abcd.supabase.co/".into(),
            anon_key: "anon".into(),
            schema: None,
        }
    }

    #[test]
    fn the_socket_lives_next_to_the_project() {
        assert_eq!(
            socket_url(&config()),
            "wss://abcd.supabase.co/realtime/v1/websocket?apikey=anon&vsn=1.0.0"
        );
        let local = Config {
            url: "http://127.0.0.1:9".into(),
            ..config()
        };
        assert!(socket_url(&local).starts_with("ws://127.0.0.1:9/realtime/v1/websocket"));
    }

    #[test]
    fn joining_listens_only_to_this_users_nudges() {
        let v: Value = serde_json::from_str(&join_message(&config(), "u1", "jwt")).unwrap();
        let change = &v["payload"]["config"]["postgres_changes"][0];
        assert_eq!(change["table"], NUDGES);
        assert_eq!(change["schema"], "public");
        assert_eq!(change["filter"], "user_id=eq.u1");
        assert_eq!(v["payload"]["access_token"], "jwt");
        let kum = Config {
            schema: Some("kum".into()),
            ..config()
        };
        let v: Value = serde_json::from_str(&join_message(&kum, "u1", "jwt")).unwrap();
        assert_eq!(
            v["payload"]["config"]["postgres_changes"][0]["schema"],
            "kum"
        );
    }

    /// Yerel bir Phoenix sunucusu taklidi: katılmayı doğrular, yanıtlar, iki dürtme gönderir.
    #[test]
    fn listening_hears_nudges_until_stopped() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let join: Value = match ws.read().unwrap() {
                Message::Text(t) => serde_json::from_str(&t).unwrap(),
                other => panic!("katılma bekleniyordu: {other:?}"),
            };
            assert_eq!(join["event"], "phx_join");
            assert_eq!(join["payload"]["access_token"], "jwt");
            let reply = r#"{"event":"phx_reply","payload":{"status":"ok","response":{}},"ref":"1","topic":"realtime:kum-nudges"}"#;
            ws.send(Message::text(reply)).unwrap();
            for writer in ["öteki", "bu"] {
                let change = json!({
                    "event": "postgres_changes",
                    "topic": TOPIC,
                    "ref": null,
                    "payload": { "data": { "type": "UPDATE", "record": { "writer": writer } } },
                });
                ws.send(Message::text(change.to_string())).unwrap();
            }
            // İstemci durdurulunca bağlantıyı kapatır.
            while !matches!(ws.read(), Ok(Message::Close(_)) | Err(_)) {}
        });
        let config = Config {
            url: format!("http://127.0.0.1:{port}"),
            anon_key: "anon".into(),
            schema: None,
        };
        let heard = std::cell::RefCell::new(Vec::<String>::new());
        let result = listen(Listen {
            config: &config,
            user_id: "u1",
            token: &mut || Some("jwt".into()),
            stop: &|| heard.borrow().len() >= 2,
            on_nudge: &mut |w| heard.borrow_mut().push(w.to_string()),
        });
        assert_eq!(result, Ok(()));
        assert_eq!(*heard.borrow(), ["öteki", "bu"]);
        peer.join().unwrap();
    }

    /// Gerçek projede bağlantı ve katılma: `KUM_RT_URL`, `KUM_RT_KEY` (anon), `KUM_RT_TOKEN`
    /// (kullanıcı ya da sunucu jetonu), `KUM_RT_USER`. Dürtme ya da 20 saniye beklenir.
    #[test]
    #[ignore = "gerçek Supabase projesi ister"]
    fn real_project_accepts_the_channel() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} gerekli"));
        let config = Config {
            url: env("KUM_RT_URL"),
            anon_key: env("KUM_RT_KEY"),
            schema: None,
        };
        let start = Instant::now();
        let heard = std::cell::RefCell::new(Vec::<String>::new());
        let token = env("KUM_RT_TOKEN");
        let result = listen(Listen {
            config: &config,
            user_id: &env("KUM_RT_USER"),
            token: &mut || Some(token.clone()),
            stop: &|| !heard.borrow().is_empty() || start.elapsed() > Duration::from_secs(20),
            on_nudge: &mut |w| heard.borrow_mut().push(w.to_string()),
        });
        eprintln!("sonuç: {result:?}, duyulan: {:?}", heard.borrow());
        assert_eq!(result, Ok(()));
    }

    /// Gerçek projede tam döngü: bir açılış `nudge` ile dürter, dinleyen duyar (ortam
    /// değişkenleri [`real_project_accepts_the_channel`]'daki gibi).
    #[test]
    #[ignore = "gerçek Supabase projesi ister"]
    fn real_project_carries_a_nudge_to_the_listener() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} gerekli"));
        let config = Config {
            url: env("KUM_RT_URL"),
            anon_key: env("KUM_RT_KEY"),
            schema: None,
        };
        let session = crate::AuthSession {
            access_token: env("KUM_RT_TOKEN"),
            refresh_token: String::new(),
            expires_at: u64::MAX,
            user_id: env("KUM_RT_USER"),
            email: String::new(),
        };
        let writer = "00000000-0000-4000-8000-0000000000e3";
        let sender = {
            let (config, session) = (config.clone(), session.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                crate::Client::new(config).remote(&session).nudge(writer)
            })
        };
        let start = Instant::now();
        let heard = std::cell::RefCell::new(Vec::<String>::new());
        let result = listen(Listen {
            config: &config,
            user_id: &session.user_id,
            token: &mut || Some(session.access_token.clone()),
            stop: &|| !heard.borrow().is_empty() || start.elapsed() > Duration::from_secs(20),
            on_nudge: &mut |w| heard.borrow_mut().push(w.to_string()),
        });
        assert_eq!(sender.join().unwrap(), Ok(()));
        assert_eq!(result, Ok(()));
        assert_eq!(*heard.borrow(), [writer]);
    }

    #[test]
    fn server_messages_are_understood() {
        assert_eq!(
            parse(
                r#"{"event":"phx_reply","payload":{"status":"ok","response":{}},"ref":"1","topic":"realtime:kum-nudges"}"#
            ),
            Event::Joined
        );
        assert_eq!(
            parse(
                r#"{"event":"phx_reply","payload":{"status":"error","response":{"reason":"Invalid JWT"}},"ref":"1"}"#
            ),
            Event::Failed("Realtime: Invalid JWT".into())
        );
        // Kalp atışının yanıtı katılma sayılmaz.
        assert_eq!(
            parse(
                r#"{"event":"phx_reply","payload":{"status":"ok","response":{}},"ref":"7","topic":"phoenix"}"#
            ),
            Event::Other
        );
        assert_eq!(
            parse(
                r#"{"event":"postgres_changes","payload":{"data":{"type":"UPDATE","table":"sync_nudges","record":{"user_id":"u1","writer":"w-2","at":"2026-10-17T10:00:00Z"}},"ids":[1]},"ref":null}"#
            ),
            Event::Nudge("w-2".into())
        );
        assert_eq!(
            parse(
                r#"{"event":"system","payload":{"status":"error","message":"Unable to subscribe"},"ref":null}"#
            ),
            Event::Failed("Realtime: Unable to subscribe".into())
        );
        assert_eq!(parse("not json"), Event::Other);
    }
}
