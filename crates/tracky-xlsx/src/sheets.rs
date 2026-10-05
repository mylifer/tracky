//! Google Sheets: kayıtlar, tabloya eklenen Apps Script web uygulaması üzerinden yazılır
//! ([`APPS_SCRIPT`]). Kum yalnızca JSON gönderir; satır bulma, ekleme ve biçim kopyalama
//! betikte, Excel aktarımıyla aynı kurallarla yapılır. Google Cloud projesi ya da OAuth
//! gerekmez: web uygulaması tablonun sahibi adına çalışır, istekler anahtarla doğrulanır.
//!
//! Betik her kaydın kimliğini saklar; yanıt yolda kaybolup aktarım yinelense de satır
//! ikinci kez yazılmaz.

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{Error, Result, Row, SheetRow, Template};

/// Tabloya eklenecek betik; `{{TOKEN}}` yerine [`script`] anahtarı koyar.
pub const APPS_SCRIPT: &str = include_str!("apps_script.gs");
/// Betiğin çalışması (her satır birkaç hücre yazar) zaman alır.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Anahtarı yerleştirilmiş betik.
pub fn script(token: &str) -> String {
    APPS_SCRIPT.replace("{{TOKEN}}", token)
}

/// Sheets'e eklemenin özeti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetAppended {
    pub filled: usize,
    pub inserted: usize,
    /// Daha önce yazıldığı için atlanan kayıt sayısı.
    pub skipped: usize,
    /// Yazılan sayfanın adı.
    pub sheet: String,
}

#[derive(Serialize)]
struct WireRow<'a> {
    id: &'a str,
    date: String,
    start: String,
    hours: f64,
    kind: &'a str,
    details: &'a str,
    party: &'a str,
    division: &'a str,
}

/// Web uygulaması adresinin biçimi: `https://script.google.com/macros/s/…/exec`
/// (Workspace hesaplarında `/a/macros/<alan>/s/…/exec`).
pub fn check_url(url: &str) -> Result<String> {
    let url = url.trim();
    let ok = url.starts_with("https://script.google.com/")
        && url.contains("/macros/")
        && url.trim_end_matches('/').ends_with("/exec");
    if ok {
        Ok(url.trim_end_matches('/').to_string())
    } else {
        Err(Error::Sheets(
            "web uygulaması adresi https://script.google.com/macros/s/…/exec biçiminde olmalı \
             (Dağıt → Dağıtımları yönet → Web uygulaması URL'si)"
                .into(),
        ))
    }
}

fn call(url: &str, body: Value) -> Result<Value> {
    let url = check_url(url)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    // Google yanıtı 302 ile başka adrese yönlendirir; ureq bunu GET ile izler (betik zaten
    // POST'ta çalışmıştır, yönlendirilen adres yalnızca sonucu verir).
    let mut resp = agent
        .post(&url)
        .send_json(body)
        .map_err(|e| Error::Sheets(format!("bağlantı hatası: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Sheets(format!("yanıt okunamadı: {e}")))?;
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Err(Error::Sheets(if status == 404 {
            "web uygulaması bulunamadı; adresi ve dağıtımı kontrol et".into()
        } else {
            format!(
                "beklenmeyen yanıt (HTTP {status}). Web uygulamasını \"Erişimi olanlar: Herkes\" \
                 ile dağıttığından ve betikte yetki verdiğinden emin ol"
            )
        }));
    };
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(value)
    } else {
        let msg = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("bilinmeyen hata");
        Err(Error::Sheets(msg.to_string()))
    }
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Tablonun düzenini doğrular ve geçmiş kayıtlardan şablon bilgilerini çıkarır.
pub fn inspect(url: &str, token: &str) -> Result<Template> {
    let v = call(url, json!({ "token": token, "action": "inspect" }))?;
    Ok(Template {
        company: string(&v, "company"),
        consultant: string(&v, "consultant"),
        parties: strings(&v, "parties"),
        divisions: strings(&v, "divisions"),
        details: strings(&v, "details"),
    })
}

/// `(kimlik, satır)` kayıtlarını tablonun ilk sayfasına ekler.
pub fn append(
    url: &str,
    token: &str,
    consultant: &str,
    rows: &[(String, Row)],
) -> Result<SheetAppended> {
    let wire: Vec<Value> = rows.iter().map(|(id, r)| wire(id, r)).collect();
    let v = call(
        url,
        json!({ "token": token, "action": "append", "consultant": consultant, "rows": wire }),
    )?;
    let count = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(SheetAppended {
        filled: count("filled"),
        inserted: count("inserted"),
        skipped: count("skipped"),
        sheet: string(&v, "sheet").unwrap_or_default(),
    })
}

/// Betik eski sürümse (yeni işlemi tanımıyorsa) güncelleme yolunu söyleyen hata.
fn outdated(e: Error) -> Error {
    match e {
        Error::Sheets(m) if m.starts_with("Bilinmeyen işlem") => Error::Sheets(
            "tablodaki betik eski; Kum'dan betiği yeniden kopyalayıp yapıştır, sonra Dağıt → \
             Dağıtımları yönet → düzenle → Yeni sürüm ile güncelle (adres değişmez)"
                .into(),
        ),
        Error::Sheets(m) if m.starts_with(CHANGED) => Error::Changed,
        e => e,
    }
}

/// Betiğin "satır değişmiş" hatasının başı.
const CHANGED: &str = "Satır değişmiş";

/// `from`–`to` (dahil) tarihli kayıt satırları, tablodaki sırayla.
pub fn list(
    url: &str,
    token: &str,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Result<Vec<SheetRow>> {
    let v = call(
        url,
        json!({ "token": token, "action": "list", "from": from.to_string(), "to": to.to_string() }),
    )
    .map_err(outdated)?;
    serde_json::from_value(v.get("rows").cloned().unwrap_or_default())
        .map_err(|e| Error::Sheets(format!("satırlar okunamadı: {e}")))
}

fn wire(id: &str, r: &Row) -> Value {
    json!(WireRow {
        id,
        date: r.date.format("%Y-%m-%d").to_string(),
        start: r.start.format("%H:%M").to_string(),
        hours: r.hours,
        kind: &r.kind,
        details: &r.details,
        party: &r.party,
        division: &r.division,
    })
}

/// `expect` satırını `row` değerleriyle değiştirir (tarih değiştiyse satır yeni gününe taşınır);
/// yazılan satırın numarası.
pub fn update(
    url: &str,
    token: &str,
    consultant: &str,
    expect: &SheetRow,
    row: &Row,
) -> Result<u32> {
    let v = call(
        url,
        json!({
            "token": token,
            "action": "update",
            "consultant": consultant,
            "expect": expect,
            "row": wire("", row),
        }),
    )
    .map_err(outdated)?;
    Ok(v.get("row").and_then(Value::as_u64).unwrap_or(0) as u32)
}

/// Tek kaydı gününe ekler (silmenin geri alınması); son aktarımın geri alma işaretlerine
/// dokunmaz. Yazılan satırın numarası.
pub fn insert(url: &str, token: &str, consultant: &str, row: &Row) -> Result<u32> {
    let v = call(
        url,
        json!({ "token": token, "action": "insert", "consultant": consultant, "row": wire("", row) }),
    )
    .map_err(outdated)?;
    Ok(v.get("row").and_then(Value::as_u64).unwrap_or(0) as u32)
}

/// `expect` satırını kaldırır (günün tek satırıysa boşaltır). `id`: satırı Kum yazdıysa kaydın
/// kimliği; betik onu unutur, kayıt yeniden gönderilebilir.
pub fn remove(url: &str, token: &str, expect: &SheetRow, id: Option<&str>) -> Result<()> {
    call(
        url,
        json!({ "token": token, "action": "remove", "expect": expect, "id": id }),
    )
    .map_err(outdated)?;
    Ok(())
}

/// Geri almanın özeti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetUndone {
    /// Silinen (aktarımın eklediği) satır sayısı.
    pub removed: usize,
    /// Boşaltılan (önceden var olan) satır sayısı.
    pub cleared: usize,
    /// Tabloda bulunamayan kayıt sayısı (elle silinmiş ya da daha sonra yeni aktarım yapılmış).
    pub missing: usize,
}

/// Son aktarımda yazılan `ids` kayıtlarının satırlarını tablodan geri alır.
pub fn undo(url: &str, token: &str, ids: &[String]) -> Result<SheetUndone> {
    let v = call(url, json!({ "token": token, "action": "undo", "ids": ids })).map_err(outdated)?;
    let count = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    Ok(SheetUndone {
        removed: count("removed"),
        cleared: count("cleared"),
        missing: count("missing"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_gets_token_and_urls_are_checked() {
        let s = script("abc123");
        assert!(s.contains(r#"const TOKEN = "abc123";"#));
        assert!(!s.contains("{{TOKEN}}"));
        assert!(check_url("https://script.google.com/macros/s/AKfy/exec").is_ok());
        assert!(check_url(" https://script.google.com/a/macros/adba.com.tr/s/AKfy/exec/ ").is_ok());
        // Tablonun kendi linki ya da test (/dev) adresi değil.
        assert!(check_url("https://docs.google.com/spreadsheets/d/1abc/edit").is_err());
        assert!(check_url("https://script.google.com/macros/s/AKfy/dev").is_err());
    }
}
