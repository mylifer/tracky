//! Yapay zekâyla zaman çizelgesi açıklaması: kullanıcının kendi Anthropic API anahtarıyla
//! Claude'a gün başına bir istek. İsteğe bağlı, varsayılan kapalı; yalnızca düğmeye basınca
//! çalışır. Anahtar yalnızca bu cihazın ayarlarında durur (eşitlemede yalnızca açık/kapalı
//! taşınır), günlüğe yazılmaz ve yalnızca api.anthropic.com'a gönderilir.
//!
//! İstek gövdesi ve yanıt ayrıştırma saf işlevlerdir ([`request_body`], [`parse_response`]);
//! satır bağlamı ve istem çekirdekte ([`tracky_core::ai`]).

use std::collections::HashMap;
use std::time::Duration;

use chrono::Days;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tracky_core::ai::{self, RowContext};
use tracky_core::timesheet::TimesheetEntry;

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Kullanılan model; tek yerde.
pub const MODEL: &str = "claude-opus-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Sunucu tarafı yedek model (ret durumunda); `"fallbacks": "default"` ile birlikte.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Opus 5'te düşünme varsayılan açıktır ve düşünme jetonları da bu sınıra sayılır: kalabalık
/// günde yanıt yarıda kalmasın. Akışsız istekte zaman aşımına düşmeyecek kadar.
const MAX_TOKENS: u32 = 16_000;
const TIMEOUT: Duration = Duration::from_secs(120);
/// Ayar anahtarı: `{ enabled, apiKey }`. `apiKey` eşitlenmez (bkz. `tracky_core::sync`).
const SETTINGS_KEY: &str = "ai_details";
/// Üslup örnekleri için geriye bakılan gün.
const EXAMPLES_LOOKBACK: u64 = 180;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AiSettings {
    enabled: bool,
    api_key: String,
}

/// Arayüze giden durum: anahtarın kendisi değil, yalnızca son dört karakteri.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    enabled: bool,
    has_key: bool,
    key_hint: Option<String>,
    model: &'static str,
}

impl From<&AiSettings> for AiStatus {
    fn from(s: &AiSettings) -> Self {
        let key = s.api_key.trim();
        let hint = (key.chars().count() >= 8).then(|| {
            let tail: String = key.chars().skip(key.chars().count() - 4).collect();
            format!("…{tail}")
        });
        Self {
            enabled: s.enabled,
            has_key: !key.is_empty(),
            key_hint: hint,
            model: MODEL,
        }
    }
}

fn settings(app: &AppHandle) -> CmdResult<AiSettings> {
    Ok(lock(&app.state::<Shared>().store)
        .setting(SETTINGS_KEY)
        .map_err(err)?
        .unwrap_or_default())
}

#[tauri::command]
pub async fn get_ai_settings(app: AppHandle) -> CmdResult<AiStatus> {
    Ok(AiStatus::from(&settings(&app)?))
}

/// Açar/kapatır; `api_key` verilirse anahtarı değiştirir (boş: siler).
#[tauri::command]
pub async fn save_ai_settings(
    app: AppHandle,
    enabled: bool,
    api_key: Option<String>,
) -> CmdResult<AiStatus> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut s: AiSettings = store
        .setting(SETTINGS_KEY)
        .map_err(err)?
        .unwrap_or_default();
    s.enabled = enabled;
    if let Some(key) = api_key {
        s.api_key = key.trim().to_string();
    }
    store.save_setting(SETTINGS_KEY, &s).map_err(err)?;
    Ok(AiStatus::from(&s))
}

/// Bağlantıyı küçük bir istekle dener; `api_key` verilmezse kayıtlı anahtar.
#[tauri::command]
pub async fn test_ai_connection(app: AppHandle, api_key: Option<String>) -> CmdResult<String> {
    let key = match api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        Some(k) => k,
        None => settings(&app)?.api_key,
    };
    if key.trim().is_empty() {
        return Err(NO_KEY.into());
    }
    let (status, body) = tauri::async_runtime::spawn_blocking(move || post(&key, &ping_body()))
        .await
        .map_err(err)??;
    check_status(status, &body)?;
    Ok(format!("Bağlantı çalışıyor ({MODEL})."))
}

const NO_KEY: &str = "Önce Anthropic API anahtarını gir (Ayarlar → Yapay zekâ).";

/// Yapay zekânın yazdığı açıklama; arayüz kaydeder (bildirimden geri alınır). Satır sayfadaki
/// anahtarıyla bulunur (canlı önerinin kimliği yoktur).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiChange {
    key: String,
    details: String,
}

/// Günün `timesheet_id` çizelgesindeki aktarılmamış satırlarına açıklama yazdırır; kaydetmez,
/// değişiklikleri döndürür. `rewrite` değilse yalnızca boş ya da otomatik (başlıklardan, hazır
/// açıklamadan) gelen açıklamalar yazılır; canlı önerilerin açıklaması hep otomatiktir, elle
/// yazılan metne dokunulmaz. Yazılacak satır yoksa istek gönderilmez.
#[tauri::command]
pub async fn ai_write_details(
    app: AppHandle,
    timesheet_id: String,
    date: String,
    rewrite: bool,
) -> CmdResult<Vec<AiChange>> {
    let settings = settings(&app)?;
    if !settings.enabled {
        return Err("Yapay zekâyla yazma kapalı (Ayarlar → Yapay zekâ).".into());
    }
    if settings.api_key.trim().is_empty() {
        return Err(NO_KEY.into());
    }
    let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|e| format!("geçersiz tarih {date}: {e}"))?;
    let (from, to) = (local_midnight(date), local_midnight(date + Days::new(1)));
    let meetings = crate::calendar::meetings(&app, from, to);

    // Bağlam depo kilidi altında toplanır; istek kilit bırakıldıktan sonra gider.
    let (keys, rows, examples) = {
        let shared = app.state::<Shared>();
        let store = lock(&shared.store);
        let ctx = store.timesheet_context().map_err(err)?;
        let sheet = ctx
            .config
            .timesheet(&timesheet_id)
            .ok_or("Zaman çizelgesi bulunamadı.")?;
        let pieces = store
            .timesheet_pieces(&ctx, from, to, &meetings)
            .map_err(err)?;
        let day = store
            .timesheet_day(&ctx, sheet, date, &pieces)
            .map_err(err)?;
        let proposals = ctx.proposals(sheet, &pieces);
        let targets: Vec<_> = day
            .rows
            .iter()
            .filter(|r| !r.exported)
            .filter(|r| {
                rewrite || r.id.is_none() || ai::is_generated(&r.entry.details, &proposals, sheet)
            })
            .collect();
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let clients: HashMap<String, String> = store
            .clients()
            .map_err(err)?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let project_clients = store.project_clients().map_err(err)?;
        let sessions = store.merged_sessions_between(from, to).map_err(err)?;
        let (known, _) = store.classify_meetings(&meetings).map_err(err)?;
        let all: Vec<TimesheetEntry> = day.rows.iter().map(|r| r.entry.clone()).collect();
        let rows: Vec<RowContext> = targets
            .iter()
            .map(|r| {
                let e = &r.entry;
                RowContext {
                    project: ctx
                        .project_name(&e.project_id)
                        .map_or_else(|| e.division.clone(), str::to_string),
                    client: project_clients
                        .get(&e.project_id)
                        .and_then(|c| clients.get(c))
                        .cloned(),
                    kind: e.kind,
                    hours: e.hours,
                    start: e.start,
                    activity: ai::row_activity(
                        e,
                        &all,
                        &sessions,
                        &known,
                        ctx.classifier(),
                        &ctx.config,
                    ),
                }
            })
            .collect();
        // Örnekler: bu günden önceki kayıtlar, yeniden eskiye.
        let mut past: Vec<TimesheetEntry> = store
            .timesheet_entries(date - Days::new(EXAMPLES_LOOKBACK), date - Days::new(1))
            .map_err(err)?
            .into_iter()
            .map(|s| s.entry)
            .collect();
        past.reverse();
        let projects: Vec<&str> = targets
            .iter()
            .map(|r| r.entry.project_id.as_str())
            .collect();
        let examples: Vec<(Option<String>, Vec<String>)> = ai::examples(&past, &projects)
            .into_iter()
            .map(|(p, texts)| {
                let name = p.map(|id| ctx.project_name(&id).map_or(id.clone(), str::to_string));
                (name, texts)
            })
            .collect();
        let keys: Vec<String> = targets.iter().map(|r| r.key.clone()).collect();
        (keys, rows, examples)
    };

    let body = request_body(ai::SYSTEM_PROMPT, &ai::user_prompt(&rows, &examples));
    let key = settings.api_key;
    let (status, text) = tauri::async_runtime::spawn_blocking(move || post(&key, &body))
        .await
        .map_err(err)??;
    let written = parse_response(status, &text, rows.len())?;
    Ok(written
        .into_iter()
        .map(|(i, details)| AiChange {
            key: keys[i].clone(),
            details,
        })
        .collect())
}

/// Yanıtın biçimi: her satır için index ve açıklama.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer" },
                        "details": { "type": "string" }
                    },
                    "required": ["index", "details"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["rows"],
        "additionalProperties": false
    })
}

/// Messages API isteği: sabit yönergeler `system`'de, günün verisi kullanıcı iletisinde;
/// yanıt JSON şemasıyla sınırlı. (Örnekleme ayarı, düşünme bütçesi ve ön doldurma bu modelde
/// reddedilir; gönderilmez.)
pub fn request_body(system: &str, user: &str) -> Value {
    json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "fallbacks": "default",
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "output_config": {
            "effort": "low",
            "format": { "type": "json_schema", "schema": schema() }
        }
    })
}

/// Bağlantı denemesi: olabildiğince küçük istek.
fn ping_body() -> Value {
    json!({
        "model": MODEL,
        "max_tokens": 16,
        "fallbacks": "default",
        "messages": [{ "role": "user", "content": "ping" }],
        "output_config": { "effort": "low" }
    })
}

/// İsteği gönderir; (HTTP durumu, gövde). Anahtar yalnızca bu başlıkta, yalnızca bu adrese gider.
fn post(key: &str, body: &Value) -> CmdResult<(u16, String)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut resp = agent
        .post(API_URL)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("anthropic-beta", FALLBACK_BETA)
        .header("content-type", "application/json")
        .send(body.to_string())
        .map_err(|e| match e {
            ureq::Error::Timeout(_) => {
                "Anthropic 2 dakikada yanıt vermedi; biraz sonra tekrar dene.".to_string()
            }
            e => format!("Anthropic'e bağlanılamadı ({e}). İnternet bağlantını kontrol et."),
        })?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Anthropic yanıtı okunamadı: {e}"))?;
    Ok((status, text))
}

/// HTTP hatasını okunur cümleye çevirir.
fn check_status(status: u16, body: &str) -> CmdResult<()> {
    if status < 400 {
        return Ok(());
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_default();
    Err(match status {
        401 => "Anthropic API anahtarı geçersiz; Ayarlar → Yapay zekâ'dan kontrol et.".into(),
        403 => {
            "Bu API anahtarının isteğe izni yok (Anthropic Console'da anahtarı kontrol et).".into()
        }
        429 => "Anthropic istek sınırına ulaşıldı; biraz bekleyip tekrar dene.".into(),
        // 529: aşırı yük.
        500..=599 => "Anthropic şu an yoğun ya da erişilemiyor; biraz sonra tekrar dene.".into(),
        _ if message.is_empty() => format!("Anthropic isteği reddetti (HTTP {status})."),
        _ => format!("Anthropic isteği reddetti (HTTP {status}): {message}"),
    })
}

#[derive(Deserialize)]
struct Written {
    rows: Vec<WrittenRow>,
}

#[derive(Deserialize)]
struct WrittenRow {
    index: i64,
    details: String,
}

/// Yanıtı (index, açıklama) listesine çevirir. Önce durma nedeni denetlenir (ret, yarıda kalma);
/// sonra ilk metin bloğu şemaya göre ayrıştırılır, diğer blok türleri atlanır. Geçersiz ya da
/// tekrarlanan index atılır, açıklamalar kurallara uydurulur ([`ai::clean_output`]).
pub fn parse_response(status: u16, body: &str, rows: usize) -> CmdResult<Vec<(usize, String)>> {
    check_status(status, body)?;
    let malformed = "Claude'un yanıtı anlaşılamadı; tekrar dene.";
    let v: Value = serde_json::from_str(body).map_err(|_| malformed.to_string())?;
    match v["stop_reason"].as_str() {
        Some("refusal") => {
            return Err(
                "Claude bu günün açıklamalarını yazmayı reddetti; açıklamaları elle yaz.".into(),
            );
        }
        Some("max_tokens") => {
            return Err("Claude'un yanıtı yarıda kaldı (satır çok fazla); tekrar dene.".into());
        }
        _ => {}
    }
    let text = v["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "text")
                .and_then(|b| b["text"].as_str())
        })
        .ok_or_else(|| malformed.to_string())?;
    let written: Written = serde_json::from_str(text).map_err(|_| malformed.to_string())?;
    let mut out: Vec<(usize, String)> = Vec::new();
    for r in written.rows {
        let Ok(i) = usize::try_from(r.index) else {
            continue;
        };
        let details = ai::clean_output(&r.details);
        if i < rows && !details.is_empty() && !out.iter().any(|(j, _)| *j == i) {
            out.push((i, details));
        }
    }
    out.sort_by_key(|(i, _)| *i);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(stop: &str, content: Value) -> String {
        json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": MODEL,
            "stop_reason": stop,
            "content": content,
        })
        .to_string()
    }

    #[test]
    fn request_has_the_expected_shape() {
        let body = request_body("yönergeler", "satırlar");
        assert_eq!(body["model"], "claude-opus-5");
        assert_eq!(body["max_tokens"], 16_000);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["system"], "yönergeler");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "satırlar");
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        let schema = &body["output_config"]["format"]["schema"];
        assert_eq!(schema["required"], json!(["rows"]));
        assert_eq!(
            schema["properties"]["rows"]["items"]["required"],
            json!(["index", "details"])
        );
        // Bu modelde reddedilen alanlar yok.
        for key in ["thinking", "temperature", "top_p", "top_k"] {
            assert!(body.get(key).is_none(), "{key}");
        }
        assert_eq!(ping_body()["model"], MODEL);
    }

    #[test]
    fn parses_rows_and_skips_other_blocks() {
        let text = json!({ "rows": [
            { "index": 1, "details": "LOY-214 ödeme ekranı düzeltmeleri." },
            { "index": 0, "details": "Sprint planlama" },
            { "index": 0, "details": "tekrar" },
            { "index": 7, "details": "olmayan satır" },
            { "index": -1, "details": "eksi" },
            { "index": 2, "details": "  " },
        ]})
        .to_string();
        let body = response(
            "end_turn",
            json!([
                { "type": "thinking", "thinking": "", "signature": "x" },
                { "type": "text", "text": text },
            ]),
        );
        assert_eq!(
            parse_response(200, &body, 3).unwrap(),
            vec![
                (0, "Sprint planlama".to_string()),
                (1, "LOY-214 ödeme ekranı düzeltmeleri".to_string()),
            ]
        );
    }

    #[test]
    fn long_results_are_truncated() {
        let text = json!({ "rows": [{ "index": 0, "details": "uzun ".repeat(40) }] }).to_string();
        let body = response("end_turn", json!([{ "type": "text", "text": text }]));
        let out = parse_response(200, &body, 1).unwrap();
        assert!(out[0].1.chars().count() <= ai::MAX_OUTPUT_CHARS);
    }

    #[test]
    fn refusal_and_max_tokens_are_errors() {
        let refused = response("refusal", json!([]));
        assert!(
            parse_response(200, &refused, 1)
                .unwrap_err()
                .contains("reddetti")
        );
        let cut = response(
            "max_tokens",
            json!([{ "type": "text", "text": "{\"rows\": [" }]),
        );
        assert!(parse_response(200, &cut, 1).unwrap_err().contains("yarıda"));
    }

    #[test]
    fn malformed_responses_are_errors() {
        let bad = "Claude'un yanıtı anlaşılamadı; tekrar dene.";
        assert_eq!(parse_response(200, "<html>", 1).unwrap_err(), bad);
        let no_text = response("end_turn", json!([{ "type": "thinking", "thinking": "" }]));
        assert_eq!(parse_response(200, &no_text, 1).unwrap_err(), bad);
        let not_json = response(
            "end_turn",
            json!([{ "type": "text", "text": "Tabii! İşte:" }]),
        );
        assert_eq!(parse_response(200, &not_json, 1).unwrap_err(), bad);
        let wrong = response(
            "end_turn",
            json!([{ "type": "text", "text": "{\"items\": []}" }]),
        );
        assert_eq!(parse_response(200, &wrong, 1).unwrap_err(), bad);
    }

    #[test]
    fn http_errors_are_friendly() {
        let api = |t: &str, m: &str| {
            json!({ "type": "error", "error": { "type": t, "message": m } }).to_string()
        };
        let e = |status, body: &str| parse_response(status, body, 1).unwrap_err();
        assert!(e(401, &api("authentication_error", "invalid x-api-key")).contains("geçersiz"));
        assert!(e(429, &api("rate_limit_error", "")).contains("sınır"));
        assert!(e(529, &api("overloaded_error", "")).contains("yoğun"));
        assert!(e(503, "").contains("yoğun"));
        assert_eq!(
            e(
                400,
                &api("invalid_request_error", "credit balance is too low")
            ),
            "Anthropic isteği reddetti (HTTP 400): credit balance is too low"
        );
    }

    #[test]
    fn status_shows_only_the_key_tail() {
        let s = AiStatus::from(&AiSettings {
            enabled: true,
            api_key: "sk-ant-api03-abcdefgh1234".into(),
        });
        assert!(s.has_key);
        assert_eq!(s.key_hint.as_deref(), Some("…1234"));
        let empty = AiStatus::from(&AiSettings::default());
        assert!(!empty.has_key && !empty.enabled && empty.key_hint.is_none());
    }
}
