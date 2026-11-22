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
use tracky_core::Store;
use tracky_core::ai::{self, RowContext};
use tracky_core::timesheet::TimesheetEntry;

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Seçilebilen modeller (Ayarlar → Yapay zekâ); ilki varsayılan. Fiyatlar milyon jeton başına
/// girdi / çıktı (Anthropic API).
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "claude-opus-5-5",
        label: "Claude Opus 5.5",
        hint: "En iyi sonuç · $4 / $20",
        fallback: true,
        price: (4.0, 20.0),
    },
    ModelInfo {
        id: "claude-sonnet-5-5",
        label: "Claude Sonnet 5.5",
        hint: "Hızlı, yarı fiyatına · $2 / $10",
        fallback: true,
        price: (2.0, 10.0),
    },
    ModelInfo {
        id: "claude-haiku-5-5",
        label: "Claude Haiku 5.5",
        hint: "En ucuz · $0,10 / $0,50",
        // Haiku'da sunucu tarafı yedek model yok; ret durumunda hata gösterilir.
        fallback: false,
        // 100 bin jetonu aşan istemde $0,50 / $2,50 ([`cost`]).
        price: (0.10, 0.50),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// Ret durumunda sunucu tarafı yedek model (`fallbacks`) desteklenir mi?
    #[serde(skip)]
    pub fallback: bool,
    /// Milyon jeton başına girdi ve çıktı fiyatı (USD).
    #[serde(skip)]
    pub price: (f64, f64),
}

/// Yanıtın `usage` alanından isteğin yaklaşık maliyeti (USD). Yanıtı yedek model verdiyse ve o
/// model listede yoksa seçilen modelin fiyatı kullanılır. Düşünme jetonları çıktıya sayılır.
pub fn cost(selected: &ModelInfo, body: &str) -> Option<f64> {
    let v: Value = serde_json::from_str(body).ok()?;
    let usage = v.get("usage")?;
    let n = |k: &str| usage[k].as_u64().unwrap_or(0) as f64;
    let served = v["model"]
        .as_str()
        .and_then(|id| MODELS.iter().find(|m| m.id == id))
        .unwrap_or(selected);
    // Önbellek okuma girdinin onda biri, yazma 1,25 katı.
    let input = n("input_tokens")
        + 0.1 * n("cache_read_input_tokens")
        + 1.25 * n("cache_creation_input_tokens");
    let (mut inp, mut out) = served.price;
    let prompt =
        n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    if served.id == "claude-haiku-5-5" && prompt > 100_000.0 {
        (inp, out) = (0.50, 2.50);
    }
    Some((input * inp + n("output_tokens") * out) / 1e6)
}

/// Bu bilgisayarda bu ay yapılan isteklerin sayısı ve yaklaşık tutarı. Eşitlenmez: her
/// bilgisayar kendi isteklerini sayar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiUsage {
    /// `YYYY-AA`; ay değişince sayaç sıfırlanır.
    month: String,
    requests: u32,
    cost_usd: f64,
}

const USAGE_KEY: &str = "ai_usage";

fn this_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

impl AiUsage {
    /// Ay değiştiyse sıfırdan başlar.
    fn current(self, month: &str) -> Self {
        if self.month == month {
            self
        } else {
            Self {
                month: month.to_string(),
                ..Self::default()
            }
        }
    }

    fn add(self, month: &str, cost: f64) -> Self {
        let mut u = self.current(month);
        u.requests += 1;
        u.cost_usd += cost;
        u
    }
}

fn usage(store: &Store) -> AiUsage {
    store
        .setting::<AiUsage>(USAGE_KEY)
        .ok()
        .flatten()
        .unwrap_or_default()
        .current(&this_month())
}

/// Ücretlendirilen (başarılı) isteği aylık sayaca ekler; isteğin tutarını döndürür.
fn record(app: &AppHandle, model: &ModelInfo, status: u16, body: &str) -> Option<f64> {
    if status >= 400 {
        return None;
    }
    let cost = cost(model, body)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let next = usage(&store).add(&this_month(), cost);
    if let Err(e) = store.save_setting(USAGE_KEY, &next) {
        log_info!("yapay zekâ kullanımı kaydedilemedi: {e}");
    }
    Some(cost)
}

/// Kayıtlı model kimliği; tanınmıyorsa ya da boşsa (eski sürümün ayarı) varsayılan.
pub fn model(id: &str) -> &'static ModelInfo {
    MODELS.iter().find(|m| m.id == id).unwrap_or(&MODELS[0])
}
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Sunucu tarafı yedek model (ret durumunda); `"fallbacks": "default"` ile birlikte.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Bu modellerde düşünme hep açıktır ve düşünme jetonları da bu sınıra sayılır: kalabalık
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
    /// Seçilen model ([`MODELS`]); boş: varsayılan. Cihazlar arasında eşitlenir.
    model: String,
}

/// Arayüze giden durum: anahtarın kendisi değil, yalnızca son dört karakteri.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    enabled: bool,
    has_key: bool,
    key_hint: Option<String>,
    model: &'static str,
    models: &'static [ModelInfo],
    /// Bu bilgisayarda bu ayki istekler ve yaklaşık tutar.
    usage: AiUsage,
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
            model: model(&s.model).id,
            models: MODELS,
            usage: AiUsage::default(),
        }
    }
}

/// Anahtarın anahtar deposundaki adı ([`crate::secrets`]).
const KEYCHAIN_KEY: &str = "ai_api_key";

fn settings(app: &AppHandle) -> CmdResult<AiSettings> {
    load_settings(&lock(&app.state::<Shared>().store))
}

/// Ayarlar ve anahtar. Veritabanında açık metin anahtar kalmışsa (eski sürüm) depoya taşınır.
fn load_settings(store: &Store) -> CmdResult<AiSettings> {
    let mut s: AiSettings = store
        .setting(SETTINGS_KEY)
        .map_err(err)?
        .unwrap_or_default();
    if s.api_key.is_empty() {
        s.api_key = crate::secrets::get_value(KEYCHAIN_KEY).unwrap_or_default();
    } else if crate::secrets::set_value(KEYCHAIN_KEY, &s.api_key) {
        let key = std::mem::take(&mut s.api_key);
        store.save_setting(SETTINGS_KEY, &s).map_err(err)?;
        log_info!("yapay zekâ anahtarı anahtar deposuna taşındı");
        s.api_key = key;
    }
    Ok(s)
}

#[tauri::command]
pub async fn get_ai_settings(app: AppHandle) -> CmdResult<AiStatus> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    Ok(AiStatus::from(&load_settings(&store)?).with_usage(usage(&store)))
}

impl AiStatus {
    fn with_usage(mut self, usage: AiUsage) -> Self {
        self.usage = usage;
        self
    }
}

/// Açar/kapatır; `api_key` verilirse anahtarı değiştirir (boş: siler), `model` verilirse
/// modeli değiştirir.
#[tauri::command]
pub async fn save_ai_settings(
    app: AppHandle,
    enabled: bool,
    api_key: Option<String>,
    model: Option<String>,
) -> CmdResult<AiStatus> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut s = load_settings(&store)?;
    s.enabled = enabled;
    if let Some(key) = api_key {
        s.api_key = key.trim().to_string();
    }
    if let Some(id) = model {
        s.model = self::model(&id).id.to_string();
    }
    // Anahtar depoya yazılabildiyse veritabanında boş kalır (depo yoksa eskisi gibi orada).
    let mut stored = s.clone();
    if crate::secrets::set_value(KEYCHAIN_KEY, &s.api_key) {
        stored.api_key.clear();
    }
    store.save_setting(SETTINGS_KEY, &stored).map_err(err)?;
    Ok(AiStatus::from(&s).with_usage(usage(&store)))
}

/// Bağlantıyı küçük bir istekle dener; `api_key` verilmezse kayıtlı anahtar.
#[tauri::command]
pub async fn test_ai_connection(app: AppHandle, api_key: Option<String>) -> CmdResult<String> {
    let saved = settings(&app)?;
    let model = model(&saved.model);
    let key = match api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    {
        Some(k) => k,
        None => saved.api_key,
    };
    if key.trim().is_empty() {
        return Err(NO_KEY.into());
    }
    let (status, body) =
        tauri::async_runtime::spawn_blocking(move || post(&key, model, &ping_body(model)))
            .await
            .map_err(err)??;
    record(&app, model, status, &body);
    check_status(status, &body)?;
    Ok(format!("Bağlantı çalışıyor ({}).", model.label))
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

/// Bir günün yazımı: değişiklikler ve isteğin yaklaşık tutarı (istek gitmediyse `None`).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiWritten {
    changes: Vec<AiChange>,
    cost_usd: Option<f64>,
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
) -> CmdResult<AiWritten> {
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
            return Ok(AiWritten {
                changes: Vec::new(),
                cost_usd: None,
            });
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

    let model = model(&settings.model);
    let body = request_body(model, ai::SYSTEM_PROMPT, &ai::user_prompt(&rows, &examples));
    let key = settings.api_key;
    let (status, text) = tauri::async_runtime::spawn_blocking(move || post(&key, model, &body))
        .await
        .map_err(err)??;
    // Yanıt ayrıştırılamasa da (ret, yarıda kalma) istek ücretlendirilmiştir.
    let cost_usd = record(&app, model, status, &text);
    let written = parse_response(status, &text, rows.len())?;
    Ok(AiWritten {
        changes: written
            .into_iter()
            .map(|(i, details)| AiChange {
                key: keys[i].clone(),
                details,
            })
            .collect(),
        cost_usd,
    })
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
pub fn request_body(model: &ModelInfo, system: &str, user: &str) -> Value {
    with_fallback(
        model,
        json!({
            "model": model.id,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
            "output_config": {
                "effort": "low",
                "format": { "type": "json_schema", "schema": schema() }
            }
        }),
    )
}

/// Model destekliyorsa ret durumunda sunucu tarafı yedek model (`FALLBACK_BETA` başlığıyla).
fn with_fallback(model: &ModelInfo, mut body: Value) -> Value {
    if model.fallback {
        body["fallbacks"] = json!("default");
    }
    body
}

/// Bağlantı denemesi: olabildiğince küçük istek.
fn ping_body(model: &ModelInfo) -> Value {
    with_fallback(
        model,
        json!({
            "model": model.id,
            "max_tokens": 16,
            "messages": [{ "role": "user", "content": "ping" }],
            "output_config": { "effort": "low" }
        }),
    )
}

/// İsteği gönderir; (HTTP durumu, gövde). Anahtar yalnızca bu başlıkta, yalnızca bu adrese gider.
fn post(key: &str, model: &ModelInfo, body: &Value) -> CmdResult<(u16, String)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut req = agent
        .post(API_URL)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json");
    if model.fallback {
        req = req.header("anthropic-beta", FALLBACK_BETA);
    }
    let mut resp = req.send(body.to_string()).map_err(|e| match e {
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
            "model": MODELS[0].id,
            "stop_reason": stop,
            "content": content,
        })
        .to_string()
    }

    #[test]
    fn request_has_the_expected_shape() {
        let body = request_body(&MODELS[0], "yönergeler", "satırlar");
        assert_eq!(body["model"], "claude-opus-5-5");
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
        assert_eq!(ping_body(&MODELS[0])["model"], "claude-opus-5-5");
    }

    #[test]
    fn model_choice_falls_back_to_the_default() {
        assert_eq!(model("claude-sonnet-5-5").id, "claude-sonnet-5-5");
        // Eski sürümün ayarında model yok; bilinmeyen kimlik de varsayılana döner.
        assert_eq!(model("").id, "claude-opus-5-5");
        assert_eq!(model("claude-opus-5").id, "claude-opus-5-5");
    }

    #[test]
    fn haiku_is_sent_without_server_side_fallback() {
        let haiku = model("claude-haiku-5-5");
        let body = request_body(haiku, "y", "s");
        assert_eq!(body["model"], "claude-haiku-5-5");
        assert!(body.get("fallbacks").is_none());
        assert!(ping_body(haiku).get("fallbacks").is_none());
        assert_eq!(
            request_body(model("claude-sonnet-5-5"), "y", "s")["fallbacks"],
            "default"
        );
    }

    #[test]
    fn cost_comes_from_usage_and_the_serving_model() {
        let body = |model: &str, input: u64, output: u64| {
            json!({ "model": model, "usage": { "input_tokens": input, "output_tokens": output } })
                .to_string()
        };
        let opus = model("claude-opus-5-5");
        let haiku = model("claude-haiku-5-5");
        // 3000 girdi × $4 + 500 çıktı × $20, milyon jeton başına.
        let c = cost(opus, &body("claude-opus-5-5", 3_000, 500)).unwrap();
        assert!((c - 0.022).abs() < 1e-9, "{c}");
        // Sonnet seçiliyken yanıtı Opus verdiyse Opus fiyatı.
        let c = cost(
            model("claude-sonnet-5-5"),
            &body("claude-opus-5-5", 1_000_000, 0),
        )
        .unwrap();
        assert!((c - 4.0).abs() < 1e-9, "{c}");
        // Listede olmayan yedek model: seçilen modelin fiyatı.
        let c = cost(haiku, &body("claude-opus-5", 50_000, 0)).unwrap();
        assert!((c - 0.005).abs() < 1e-9, "{c}");
        // Haiku'da 100 bin jetonu aşan istem daha pahalı.
        let c = cost(haiku, &body("claude-haiku-5-5", 200_000, 0)).unwrap();
        assert!((c - 0.10).abs() < 1e-9, "{c}");
        let cached = json!({ "model": "claude-opus-5-5", "usage": {
            "input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 1_000_000
        }})
        .to_string();
        assert!((cost(opus, &cached).unwrap() - 0.4).abs() < 1e-9);
        assert_eq!(cost(opus, "{}"), None);
        assert_eq!(cost(opus, "geçersiz"), None);
    }

    #[test]
    fn usage_counts_this_month_and_resets_on_a_new_one() {
        let u = AiUsage::default().add("2026-11", 0.02).add("2026-11", 0.03);
        assert_eq!(u.requests, 2);
        assert!((u.cost_usd - 0.05).abs() < 1e-9);
        let next = u.clone().add("2026-12", 0.01);
        assert_eq!((next.month.as_str(), next.requests), ("2026-12", 1));
        assert_eq!(u.clone().current("2026-11"), u);
        assert_eq!(u.current("2026-12").requests, 0);
    }

    #[test]
    fn settings_without_a_model_still_load() {
        let s: AiSettings = serde_json::from_value(json!({ "enabled": true })).unwrap();
        assert_eq!(model(&s.model).id, "claude-opus-5-5");
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
            model: String::new(),
        });
        assert!(s.has_key);
        assert_eq!(s.key_hint.as_deref(), Some("…1234"));
        let empty = AiStatus::from(&AiSettings::default());
        assert!(!empty.has_key && !empty.enabled && empty.key_hint.is_none());
    }
}
