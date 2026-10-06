//! Google Sheets API (v4) ile doğrudan: Apps Script'e göre çok daha hızlı (istek başına
//! saniyeler değil, yüz milisaniyeler). Kullanıcının Google hesabının OAuth erişim anahtarıyla
//! çalışır (anahtarı uygulama alır ve yeniler).
//!
//! Kurallar betikle ([`crate::sheets`]) ve Excel'le aynıdır: tablonun ilk sayfası, sütunlar
//! başlıklardan; önce günün önceden doldurulmuş boş satırı, yoksa gün ve saat sırasını bozmayan
//! yere (biçimi üstteki satırdan) eklenen satır ([`crate::slot`]); satır beklenen eski içeriğiyle bulunur. Son aktarımın
//! satırları betikle aynı biçimde işaretlenir (`kum_row` geliştirici meta verisi): iki yol
//! birbirinin aktarımını geri alabilir.
//!
//! Her işlem tablonun yerel bir kopyası üzerinde planlanır ([`Sheet`]) ve tek toplu istekle
//! (`batchUpdate`) yazılır.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate, NaiveTime};
use serde_json::{Value, json};

use crate::sheets::{SheetAppended, SheetUndone};
use crate::{
    Columns, DAY_FORMULA, Error, HEADER_ROW, Line, Result, Row, SheetRow, Template, cell_hours,
    cell_time, columns_of, company_of, date_to_serial, in_order, ranked, serial_to_date, slot,
    time_to_fraction,
};

const API: &str = "https://sheets.googleapis.com/v4/spreadsheets";
const TIMEOUT: Duration = Duration::from_secs(60);
/// Son aktarımın satır işareti (betikle aynı): değeri "<kimlik>|<1: eklendi, 0: dolduruldu>".
const ROW_KEY: &str = "kum_row";
/// İlk sayfanın kimliği ve adı bu süre saklanır (her istekte sorulmasın); kısa: sayfalar
/// yeniden sıralanınca ya da yeniden adlandırılınca eski sayfaya yazılmasın.
const SHEET_TTL: Duration = Duration::from_secs(45);

/// Tablo bağlantısından (`https://docs.google.com/spreadsheets/d/<kimlik>/edit…`) kimlik.
pub fn spreadsheet_id(link: &str) -> Option<String> {
    let rest = link.split("/spreadsheets/d/").nth(1)?;
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (id.len() >= 20).then_some(id)
}

fn agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into()
    })
}

/// Yol parçası için yüzde kodlama (sayfa adında boşluk, tırnak, ünlem olabilir).
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn call(token: &str, method: &str, url: &str, body: Option<Value>) -> Result<Value> {
    let auth = format!("Bearer {token}");
    let sent = match (method, body) {
        ("GET", _) => agent().get(url).header("Authorization", &auth).call(),
        (_, body) => agent()
            .post(url)
            .header("Authorization", &auth)
            .send_json(body.unwrap_or_else(|| json!({}))),
    };
    let mut resp = sent.map_err(|e| Error::Sheets(format!("bağlantı hatası: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Sheets(format!("yanıt okunamadı: {e}")))?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status < 400 {
        return Ok(value);
    }
    let msg = value["error"]["message"]
        .as_str()
        .unwrap_or("bilinmeyen hata")
        .to_string();
    Err(Error::Sheets(match status {
        401 => "Google oturumu geçersiz; Ayarlar → Zaman çizelgeleri'nden Google'a yeniden bağlan"
            .into(),
        403 => forbidden(&value, &msg),
        404 => "tablo bulunamadı; tablonun bağlantısını kontrol et".into(),
        429 => "Google istek sınırına ulaşıldı; biraz sonra tekrar dene".into(),
        _ => format!("HTTP {status}: {msg}"),
    }))
}

/// 403'ün nedeni: Google aynı kodla üç ayrı durumu bildirir; ayrıntılardaki `reason` ayırır.
fn forbidden(value: &Value, msg: &str) -> String {
    let reasons: Vec<&str> = value["error"]["details"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["reason"].as_str())
        .collect();
    if reasons.contains(&"SERVICE_DISABLED") {
        "Google Cloud projende Sheets API kapalı; Cloud Console → API'ler ve Hizmetler'den \
         Google Sheets API'yi etkinleştir"
            .into()
    } else if reasons.contains(&"ACCESS_TOKEN_SCOPE_INSUFFICIENT") {
        "Google'a bağlanırken tablolara erişim izni verilmedi; Ayarlar → Zaman çizelgeleri'nden \
         yeniden bağlan ve onay ekranında Google E-Tablolar iznini işaretle"
            .into()
    } else {
        format!(
            "bağlı Google hesabının bu tabloda düzenleme yetkisi yok; tabloyu bu hesapla açıp \
             Düzenleyen olduğunu kontrol et ya da doğru hesapla yeniden bağlan ({msg})"
        )
    }
}

/// Tablonun ilk sayfası ve formül ayracı.
#[derive(Debug, Clone)]
struct FirstSheet {
    sheet_id: i64,
    title: String,
    /// Tablonun dil ayarında ondalık ayracı virgül: formül bağımsız değişkenleri noktalı
    /// virgülle ayrılır (tr_TR, de_DE…). API'ye yazılan formül bu ayara göre okunur.
    semicolons: bool,
}

/// Tablo kimliği → (okunma anı, ilk sayfa).
type SheetCache = HashMap<String, (Instant, FirstSheet)>;

/// Ondalık ayracı virgül olan diller (formülde `;`).
const SEMICOLON_LANGS: &[&str] = &[
    "tr", "de", "fr", "es", "it", "pt", "nl", "ru", "pl", "sv", "da", "fi", "nb", "no", "cs", "sk",
    "hu", "ro", "el", "uk", "bg", "hr", "sl", "sr", "lt", "lv", "et", "id", "vi", "ca", "is",
];

fn semicolon_locale(locale: &str) -> bool {
    let lang = locale.split(['_', '-']).next().unwrap_or("").to_lowercase();
    SEMICOLON_LANGS.contains(&lang.as_str())
}

/// Tablonun ilk sayfası (betikteki `getSheets()[0]`) ve dil ayarı.
fn first_sheet(token: &str, id: &str) -> Result<FirstSheet> {
    static CACHE: Mutex<Option<SheetCache>> = Mutex::new(None);
    if let Some((at, first)) = CACHE
        .lock()
        .ok()
        .and_then(|c| c.as_ref().and_then(|m| m.get(id).cloned()))
        && at.elapsed() < SHEET_TTL
    {
        return Ok(first);
    }
    let v = call(
        token,
        "GET",
        &format!("{API}/{id}?fields=properties.locale,sheets.properties(sheetId,title,index)"),
        None,
    )?;
    let sheet = v["sheets"]
        .as_array()
        .and_then(|s| s.iter().min_by_key(|s| s["properties"]["index"].as_i64()))
        .ok_or(Error::NoSheet)?;
    let first = FirstSheet {
        sheet_id: sheet["properties"]["sheetId"].as_i64().unwrap_or(0),
        title: sheet["properties"]["title"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        semicolons: semicolon_locale(v["properties"]["locale"].as_str().unwrap_or("")),
    };
    if let Ok(mut c) = CACHE.lock() {
        c.get_or_insert_with(HashMap::new)
            .insert(id.to_string(), (Instant::now(), first.clone()));
    }
    Ok(first)
}

/// Sayfa adı A1 gösteriminde ('Mart 2026').
fn quoted(title: &str) -> String {
    format!("'{}'", title.replace('\'', "''"))
}

/// Sütun numarası (1'den) → harf (1 → A, 27 → AA).
fn letter(mut c: u32) -> String {
    let mut s = Vec::new();
    while c > 0 {
        let r = ((c - 1) % 26) as u8;
        s.push(b'A' + r);
        c = (c - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// Hücre değeri metin olarak (sayı gereksiz sıfırsız: 1, 1.5).
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        _ => String::new(),
    }
}

fn blank(v: &Value) -> bool {
    text(v).is_empty()
}

/// Son aktarımın bir satır işareti.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Marker {
    meta_id: i64,
    /// Kaydın kimliği (ilk 13 karakter, betikle aynı).
    key: String,
    fresh: bool,
    row: u32,
}

/// Tablonun ilk sayfasının yerel kopyası ve yazılacak istekler: satır ekleme, silme ve hücre
/// yazma hem kopyaya uygulanır hem isteğe eklenir; sonraki adımlar güncel satırlarla hesaplanır.
pub(crate) struct Sheet {
    sheet_id: i64,
    title: String,
    /// Satırlar (0: başlık), hücreler biçimsiz değerleriyle; sondaki boşlar yok.
    grid: Vec<Vec<Value>>,
    cols: Columns,
    /// Formülde bağımsız değişken ayracı `;` (tablonun dil ayarı).
    semicolons: bool,
    /// Day sütununun formülleri (satır sırasıyla); gerektiğinde okunur.
    day_formulas: Option<Vec<String>>,
    requests: Vec<Value>,
}

impl Sheet {
    pub(crate) fn new(sheet_id: i64, title: String, grid: Vec<Vec<Value>>) -> Result<Self> {
        let headers: Vec<String> = grid
            .first()
            .map(|r| r.iter().map(text).collect())
            .unwrap_or_default();
        Ok(Self {
            cols: columns_of(&headers)?,
            semicolons: false,
            sheet_id,
            title,
            grid,
            day_formulas: None,
            requests: Vec::new(),
        })
    }

    fn open(token: &str, id: &str) -> Result<Self> {
        let FirstSheet {
            sheet_id: sid,
            title,
            semicolons,
        } = first_sheet(token, id)?;
        let v = call(
            token,
            "GET",
            &format!(
                "{API}/{id}/values/{}?valueRenderOption=UNFORMATTED_VALUE&dateTimeRenderOption=SERIAL_NUMBER",
                encode(&quoted(&title))
            ),
            None,
        )?;
        let grid = v["values"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| r.as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        let mut sheet = Self::new(sid, title, grid)?;
        sheet.semicolons = semicolons;
        Ok(sheet)
    }

    /// Day sütununun formüllerini okur (yeni satırın Day hücresi tablonun yöntemiyle dolsun).
    /// Satır ekleyip silmeden önce çağrılmalı: sonraki değişiklikler kopyaya da uygulanır.
    fn load_formulas(&mut self, token: &str, id: &str) -> Result<()> {
        let Some(day) = self.cols.day else {
            return Ok(());
        };
        if self.day_formulas.is_some() {
            return Ok(());
        }
        let col = letter(day);
        let v = call(
            token,
            "GET",
            &format!(
                "{API}/{id}/values/{}?valueRenderOption=FORMULA&majorDimension=COLUMNS",
                encode(&format!("{}!{col}:{col}", quoted(&self.title)))
            ),
            None,
        )?;
        let mut f: Vec<String> = v["values"][0]
            .as_array()
            .map(|c| c.iter().map(text).collect())
            .unwrap_or_default();
        f.resize(self.grid.len(), String::new());
        self.day_formulas = Some(f);
        Ok(())
    }

    fn last_row(&self) -> u32 {
        self.grid.len() as u32
    }

    fn cell(&self, c: u32, r: u32) -> &Value {
        static NULL: Value = Value::Null;
        self.grid
            .get(r as usize - 1)
            .and_then(|row| row.get(c as usize - 1))
            .unwrap_or(&NULL)
    }

    fn set(&mut self, c: u32, r: u32, v: Value) {
        let (r, c) = (r as usize - 1, c as usize - 1);
        if self.grid.len() <= r {
            self.grid.resize(r + 1, Vec::new());
        }
        let row = &mut self.grid[r];
        if row.len() <= c {
            row.resize(c + 1, Value::Null);
        }
        row[c] = v;
    }

    fn date_at(&self, r: u32) -> Option<NaiveDate> {
        match self.cell(self.cols.date, r) {
            Value::Number(n) => serial_to_date(&n.to_string()),
            _ => None,
        }
    }

    /// Satır → kayıt; tarihsiz ya da boş (önceden doldurulmuş gün satırı) ise `None`.
    fn read_row(&self, r: u32) -> Option<SheetRow> {
        let date = self.date_at(r)?;
        let cols = self.cols;
        let t = |c: u32| text(self.cell(c, r));
        let row = SheetRow {
            row: r,
            date,
            start: match self.cell(cols.start, r) {
                Value::Null => None,
                v => cell_time(&text(v)),
            },
            hours: match self.cell(cols.hours, r) {
                Value::Number(n) => n.as_f64(),
                v => cell_hours(&text(v)),
            },
            kind: t(cols.kind),
            details: t(cols.details),
            party: t(cols.party),
            division: t(cols.division),
            consultant: cols.consultant.map(t).unwrap_or_default(),
        };
        let empty = row.kind.is_empty() && row.details.is_empty() && row.hours.is_none();
        (!empty).then_some(row)
    }

    fn rows(&self) -> impl Iterator<Item = SheetRow> + '_ {
        ((HEADER_ROW + 1)..=self.last_row()).filter_map(|r| self.read_row(r))
    }

    /// Beklenen kaydın bugünkü satırı: ipucu hâlâ aynıysa o, değilse içeriği aynı tek satır.
    fn locate(&self, expect: &SheetRow) -> Result<u32> {
        let at = |r: u32| self.read_row(r).is_some_and(|s| s.same(expect));
        if expect.row > HEADER_ROW && at(expect.row) {
            return Ok(expect.row);
        }
        let found: Vec<u32> = ((HEADER_ROW + 1)..=self.last_row())
            .filter(|&r| at(r))
            .collect();
        match found[..] {
            [r] => Ok(r),
            _ => Err(Error::Changed),
        }
    }

    fn range(&self, r: u32, c: u32) -> Value {
        json!({
            "sheetId": self.sheet_id,
            "startRowIndex": r - 1, "endRowIndex": r,
            "startColumnIndex": c - 1, "endColumnIndex": c,
        })
    }

    /// Hücreye yazar: `data` hücre verisi (`userEnteredValue`, biçim), `fields` değişen alanlar.
    fn write(&mut self, r: u32, c: u32, data: Value, fields: &str, local: Value) {
        self.requests.push(json!({ "updateCells": {
            "rows": [{ "values": [data] }],
            "fields": fields,
            "start": { "sheetId": self.sheet_id, "rowIndex": r - 1, "columnIndex": c - 1 },
        }}));
        self.set(c, r, local);
    }

    fn write_text(&mut self, r: u32, c: u32, s: &str) {
        self.write(
            r,
            c,
            json!({ "userEnteredValue": { "stringValue": s } }),
            "userEnteredValue",
            Value::String(s.to_string()),
        );
    }

    /// `at` numaralı yeni satır ekler; biçimini üstteki satırdan alır (başlık değilse).
    fn insert_row(&mut self, at: u32) {
        self.requests.push(json!({ "insertDimension": {
            "range": { "sheetId": self.sheet_id, "dimension": "ROWS", "startIndex": at - 1, "endIndex": at },
            "inheritFromBefore": at - 1 > HEADER_ROW,
        }}));
        let i = (at as usize - 1).min(self.grid.len());
        self.grid.insert(i, Vec::new());
        if let Some(f) = &mut self.day_formulas {
            f.insert(i.min(f.len()), String::new());
        }
    }

    fn delete_row(&mut self, r: u32) {
        self.requests.push(json!({ "deleteDimension": {
            "range": { "sheetId": self.sheet_id, "dimension": "ROWS", "startIndex": r - 1, "endIndex": r },
        }}));
        if (r as usize) <= self.grid.len() {
            self.grid.remove(r as usize - 1);
        }
        if let Some(f) = &mut self.day_formulas
            && (r as usize) <= f.len()
        {
            f.remove(r as usize - 1);
        }
    }

    fn lines(&self) -> Vec<Line> {
        ((HEADER_ROW + 1)..=self.last_row())
            .map(|r| match self.date_at(r) {
                None => Line::Undated,
                Some(d) => self
                    .read_row(r)
                    .map_or(Line::Blank(d), |e| Line::Entry(d, e.start)),
            })
            .collect()
    }

    /// `date` günü `start` saatli kaydın satırı ([`slot`]): gün ve saat sırası bozulmaz.
    /// (satır, eklendi mi)
    fn place(&mut self, date: NaiveDate, start: NaiveTime) -> (u32, bool) {
        let (r, fresh) = slot(&self.lines(), date, start);
        if fresh {
            self.insert_row(r);
        }
        (r, fresh)
    }

    /// `r` satırındaki `date` günlü kaydı kaldırır: günün başka satırı varsa satır silinir
    /// (`true`), yoksa gün satırı kalır ve kayıt hücreleri boşaltılır.
    fn vacate(&mut self, r: u32, date: NaiveDate) -> bool {
        let others =
            ((HEADER_ROW + 1)..=self.last_row()).any(|x| x != r && self.date_at(x) == Some(date));
        if others {
            self.delete_row(r);
        } else {
            let c = self.cols;
            for col in [c.start, c.hours, c.kind, c.details, c.party, c.division] {
                self.write(r, col, json!({}), "userEnteredValue", Value::Null);
            }
        }
        others
    }

    /// Kaydı `r` satırına yazar (betikteki `put_` ve `day_`).
    fn put_row(&mut self, r: u32, consultant: &str, row: &Row) {
        let c = self.cols;
        let serial = date_to_serial(row.date);
        self.write(
            r,
            c.date,
            json!({
                "userEnteredValue": { "numberValue": serial },
                "userEnteredFormat": { "numberFormat": { "type": "DATE", "pattern": "d/m/yy" } },
            }),
            "userEnteredValue,userEnteredFormat.numberFormat",
            json!(serial),
        );
        if let Some(col) = c.consultant
            && !consultant.trim().is_empty()
        {
            self.write_text(r, col, consultant.trim());
        }
        let start = time_to_fraction(row.start);
        self.write(
            r,
            c.start,
            json!({
                "userEnteredValue": { "numberValue": start },
                "userEnteredFormat": { "numberFormat": { "type": "TIME", "pattern": "hh:mm" } },
            }),
            "userEnteredValue,userEnteredFormat.numberFormat",
            json!(start),
        );
        // Otomatik biçim: 1, 0,5, 0,25 (gereksiz sıfırlar olmadan).
        self.write(
            r,
            c.hours,
            json!({ "userEnteredValue": { "numberValue": row.hours }, "userEnteredFormat": {} }),
            "userEnteredValue,userEnteredFormat.numberFormat",
            json!(row.hours),
        );
        self.write_text(r, c.kind, &row.kind);
        self.write_text(r, c.details, &row.details);
        self.write_text(r, c.party, &row.party);
        self.write_text(r, c.division, &row.division);
        self.day(r, row.date);
    }

    /// Day hücresi tablonun kendi yöntemiyle: sütunda ARRAYFORMULA varsa dokunulmaz; yoksa
    /// üstteki ilk formül göreli kopyalanır, formül yoksa üstteki değer türünde değer yazılır,
    /// hiçbiri yoksa haftanın günü formülü. Hücre doluysa dokunulmaz.
    fn day(&mut self, r: u32, date: NaiveDate) {
        let Some(col) = self.cols.day else { return };
        let Some(formulas) = self.day_formulas.clone() else {
            // Formüller okunmadı: hücre zaten doluysa sorun yok; boşsa dokunulmaz.
            return;
        };
        let f = |row: u32| formulas.get(row as usize - 1).map_or("", |s| s.as_str());
        if !blank(self.cell(col, r)) || f(r).starts_with('=') {
            return;
        }
        if formulas
            .iter()
            .any(|x| x.to_lowercase().contains("arrayformula"))
        {
            return;
        }
        for i in ((HEADER_ROW + 1)..r).rev() {
            if f(i).starts_with('=') {
                self.requests.push(json!({ "copyPaste": {
                    "source": self.range(i, col),
                    "destination": self.range(r, col),
                    "pasteType": "PASTE_FORMULA",
                    "pasteOrientation": "NORMAL",
                }}));
                self.mark_formula(r, f(i).to_string());
                return;
            }
            match self.cell(col, i).clone() {
                Value::Number(_) => {
                    let serial = date_to_serial(date);
                    self.write(
                        r,
                        col,
                        json!({ "userEnteredValue": { "numberValue": serial } }),
                        "userEnteredValue",
                        json!(serial),
                    );
                    return;
                }
                v if !blank(&v) => {
                    self.write_text(r, col, &date.format("%A").to_string());
                    return;
                }
                _ => {}
            }
        }
        let mut formula = format!(
            "={}",
            DAY_FORMULA.replace("B{r}", &format!("{}{r}", letter(self.cols.date)))
        );
        if self.semicolons {
            formula = formula.replace(',', ";");
        }
        self.write(
            r,
            col,
            json!({ "userEnteredValue": { "formulaValue": formula } }),
            "userEnteredValue",
            Value::String(date.weekday().to_string()),
        );
        self.mark_formula(r, formula);
    }

    fn mark_formula(&mut self, r: u32, formula: String) {
        if let Some(f) = &mut self.day_formulas {
            if f.len() < r as usize {
                f.resize(r as usize, String::new());
            }
            f[r as usize - 1] = formula;
        }
    }

    fn mark(&mut self, r: u32, key: &str, fresh: bool) {
        self.requests.push(json!({ "createDeveloperMetadata": { "developerMetadata": {
            "metadataKey": ROW_KEY,
            "metadataValue": format!("{key}|{}", u8::from(fresh)),
            "location": { "dimensionRange": {
                "sheetId": self.sheet_id, "dimension": "ROWS", "startIndex": r - 1, "endIndex": r,
            }},
            "visibility": "DOCUMENT",
        }}}));
    }

    fn unmark(&mut self, meta_id: i64) {
        self.requests
            .push(json!({ "deleteDeveloperMetadata": { "dataFilter": {
                "developerMetadataLookup": { "metadataId": meta_id },
            }}}));
    }

    fn commit(self, token: &str, id: &str) -> Result<()> {
        if self.requests.is_empty() {
            return Ok(());
        }
        call(
            token,
            "POST",
            &format!("{API}/{id}:batchUpdate"),
            Some(json!({ "requests": self.requests })),
        )?;
        Ok(())
    }

    // --- planlar (ağsız; testlerde doğrudan denenir) ---

    fn plan_update(&mut self, consultant: &str, expect: &SheetRow, row: &Row) -> Result<u32> {
        let mut r = self.locate(expect)?;
        let i = (r - HEADER_ROW - 1) as usize;
        if row.date != expect.date || !in_order(&self.lines(), i, row.date, row.start) {
            self.vacate(r, expect.date);
            r = self.place(row.date, row.start).0;
        }
        self.put_row(r, consultant, row);
        Ok(r)
    }

    fn plan_insert(&mut self, consultant: &str, row: &Row) -> u32 {
        let r = self.place(row.date, row.start).0;
        self.put_row(r, consultant, row);
        r
    }

    /// Kaydı kaldırır; boşaltılan (silinmeyen) satırın işaretleri de kalkar.
    fn plan_remove(&mut self, expect: &SheetRow, markers: &[Marker]) -> Result<()> {
        let r = self.locate(expect)?;
        if !self.vacate(r, expect.date) {
            for m in markers.iter().filter(|m| m.row == r) {
                self.unmark(m.meta_id);
            }
        }
        Ok(())
    }

    /// Kayıtları ekler. Yalnızca son aktarım geri alınabilir: önceki işaretler kalkar. Tabloda
    /// içeriği aynı satır varsa (yanıtı kaybolan aktarımın yinelenmesi) kayıt atlanır.
    fn plan_append(
        &mut self,
        consultant: &str,
        rows: &[(String, Row)],
        markers: &[Marker],
    ) -> SheetAppended {
        for m in markers {
            self.unmark(m.meta_id);
        }
        let mut sorted: Vec<&(String, Row)> = rows.iter().collect();
        sorted.sort_by_key(|(_, r)| (r.date, r.start));
        let (mut filled, mut inserted, mut skipped) = (0, 0, 0);
        for (id, row) in sorted {
            let want = SheetRow {
                row: 0,
                date: row.date,
                start: Some(row.start),
                hours: Some(row.hours),
                kind: row.kind.clone(),
                details: row.details.clone(),
                party: row.party.clone(),
                division: row.division.clone(),
                // Başka danışmanın aynı içerikli satırı bizimkinin yerine geçmesin.
                consultant: consultant.trim().to_string(),
            };
            if self.rows().any(|s| s.same(&want)) {
                skipped += 1;
                continue;
            }
            let (r, fresh) = self.place(row.date, row.start);
            if fresh {
                inserted += 1;
            } else {
                filled += 1;
            }
            self.put_row(r, consultant, row);
            self.mark(r, &id.chars().take(13).collect::<String>(), fresh);
        }
        SheetAppended {
            filled,
            inserted,
            skipped,
            sheet: self.title.clone(),
        }
    }

    /// Son aktarımın `ids` satırlarını geri alır: eklenen satır silinir, önceden var olan boş
    /// satırın yazılan hücreleri boşaltılır.
    fn plan_undo(&mut self, ids: &[String], markers: &[Marker]) -> SheetUndone {
        let want: std::collections::HashSet<String> =
            ids.iter().map(|i| i.chars().take(13).collect()).collect();
        let mut hits: Vec<&Marker> = markers.iter().filter(|m| want.contains(&m.key)).collect();
        // Alttan yukarı: silinen satır üsttekilerin yerini kaydırmasın.
        hits.sort_by_key(|m| std::cmp::Reverse(m.row));
        let (mut removed, mut cleared) = (0, 0);
        for m in &hits {
            if m.fresh {
                self.delete_row(m.row);
                removed += 1;
            } else {
                let c = self.cols;
                for col in [c.start, c.hours, c.kind, c.details, c.party, c.division] {
                    self.write(m.row, col, json!({}), "userEnteredValue", Value::Null);
                }
                self.unmark(m.meta_id);
                cleared += 1;
            }
        }
        let found: std::collections::HashSet<&str> = hits.iter().map(|m| m.key.as_str()).collect();
        let undone = ids
            .iter()
            .filter(|i| found.contains(i.chars().take(13).collect::<String>().as_str()))
            .cloned()
            .collect();
        SheetUndone {
            removed,
            cleared,
            missing: want.len() - found.len(),
            undone,
        }
    }
}

/// Bu sayfadaki son aktarım işaretleri.
fn markers(token: &str, id: &str, sheet_id: i64) -> Result<Vec<Marker>> {
    let v = call(
        token,
        "POST",
        &format!("{API}/{id}/developerMetadata:search"),
        Some(json!({ "dataFilters": [{ "developerMetadataLookup": { "metadataKey": ROW_KEY } }] })),
    )?;
    Ok(v["matchedDeveloperMetadata"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|m| {
                    let m = &m["developerMetadata"];
                    let range = &m["location"]["dimensionRange"];
                    if range["sheetId"].as_i64().unwrap_or(0) != sheet_id {
                        return None;
                    }
                    let (key, fresh) = m["metadataValue"].as_str()?.split_once('|')?;
                    Some(Marker {
                        meta_id: m["metadataId"].as_i64()?,
                        key: key.to_string(),
                        fresh: fresh == "1",
                        row: range["startIndex"].as_u64()? as u32 + 1,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// `from`–`to` (dahil) tarihli kayıt satırları, tablodaki sırayla.
pub fn list(token: &str, id: &str, from: NaiveDate, to: NaiveDate) -> Result<Vec<SheetRow>> {
    let sheet = Sheet::open(token, id)?;
    Ok(sheet
        .rows()
        .filter(|s| (from..=to).contains(&s.date))
        .collect())
}

/// `expect` satırını `row` değerleriyle değiştirir (tarih değiştiyse satır yeni gününe taşınır);
/// yazılan satırın numarası.
pub fn update(
    token: &str,
    id: &str,
    consultant: &str,
    expect: &SheetRow,
    row: &Row,
) -> Result<u32> {
    let mut sheet = Sheet::open(token, id)?;
    // Satır taşınabilir (yeni gün ya da gün içinde sırası bozan saat): Day formülü taşınmadan
    // önce okunmalı; sonradan okunan formüller satırların eski yerlerine göredir.
    if row.date != expect.date || expect.start != Some(row.start) {
        sheet.load_formulas(token, id)?;
    }
    let r = sheet.plan_update(consultant, expect, row)?;
    // Yerinde değişiklikte Day hücresi boşsa formüller sonradan okunur (satır kaymadı).
    if row.date == expect.date
        && sheet.cols.day.is_some_and(|d| blank(sheet.cell(d, r)))
        && sheet.day_formulas.is_none()
    {
        sheet.load_formulas(token, id)?;
        sheet.day(r, row.date);
    }
    sheet.commit(token, id)?;
    Ok(r)
}

/// Tek kaydı gününe ekler (silmenin geri alınması); son aktarımın işaretlerine dokunmaz.
pub fn insert(token: &str, id: &str, consultant: &str, row: &Row) -> Result<u32> {
    let mut sheet = Sheet::open(token, id)?;
    sheet.load_formulas(token, id)?;
    let r = sheet.plan_insert(consultant, row);
    sheet.commit(token, id)?;
    Ok(r)
}

/// `expect` satırını kaldırır (günün tek satırıysa boşaltır).
pub fn remove(token: &str, id: &str, expect: &SheetRow) -> Result<()> {
    let mut sheet = Sheet::open(token, id)?;
    let found = markers(token, id, sheet.sheet_id)?;
    sheet.plan_remove(expect, &found)?;
    sheet.commit(token, id)
}

/// `(kimlik, satır)` kayıtlarını ekler.
pub fn append(
    token: &str,
    id: &str,
    consultant: &str,
    rows: &[(String, Row)],
) -> Result<SheetAppended> {
    let mut sheet = Sheet::open(token, id)?;
    sheet.load_formulas(token, id)?;
    let found = markers(token, id, sheet.sheet_id)?;
    let done = sheet.plan_append(consultant, rows, &found);
    sheet.commit(token, id)?;
    Ok(done)
}

/// Son aktarımda yazılan `ids` kayıtlarının satırlarını geri alır.
pub fn undo(token: &str, id: &str, ids: &[String]) -> Result<SheetUndone> {
    let mut sheet = Sheet::open(token, id)?;
    let found = markers(token, id, sheet.sheet_id)?;
    let done = sheet.plan_undo(ids, &found);
    sheet.commit(token, id)?;
    Ok(done)
}

/// Tablonun düzenini doğrular ve geçmiş kayıtlardan şablon bilgilerini çıkarır.
pub fn inspect(token: &str, id: &str) -> Result<Template> {
    let sheet = Sheet::open(token, id)?;
    let cols = sheet.cols;
    let column =
        |c: u32| ranked(((HEADER_ROW + 1)..=sheet.last_row()).map(|r| text(sheet.cell(c, r))));
    Ok(Template {
        company: company_of(&text(sheet.cell(cols.division, HEADER_ROW))),
        consultant: cols.consultant.and_then(|c| column(c).into_iter().next()),
        parties: column(cols.party),
        divisions: column(cols.division),
        details: column(cols.details),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_reasons() {
        let err = |reason: Option<&str>| {
            let details = reason.map_or(
                json!([]),
                |r| json!([{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": r}]),
            );
            json!({"error": {"code": 403, "status": "PERMISSION_DENIED", "details": details}})
        };
        let msg = "The caller does not have permission";
        assert!(forbidden(&err(Some("SERVICE_DISABLED")), msg).contains("Sheets API kapalı"));
        assert!(
            forbidden(&err(Some("ACCESS_TOKEN_SCOPE_INSUFFICIENT")), msg)
                .contains("izni verilmedi")
        );
        let plain = forbidden(&err(None), msg);
        assert!(plain.contains("düzenleme yetkisi yok") && plain.contains(msg));
        assert!(forbidden(&Value::Null, msg).contains("düzenleme yetkisi yok"));
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn row(day: u32, hh: u32, hours: f64, details: &str) -> Row {
        Row {
            date: d(day),
            start: NaiveTime::from_hms_opt(hh, 0, 0).unwrap(),
            hours,
            kind: "Working".into(),
            details: details.into(),
            party: "ADBA".into(),
            division: "Trumore".into(),
        }
    }

    /// Şablonun düzeni: başlık; 1 Eki dolu, 2 ve 3 Eki önceden doldurulmuş boş satırlar.
    fn sheet() -> Sheet {
        let s = |v: &str| json!(v);
        let n = |day: u32| json!(date_to_serial(d(day)));
        let grid = vec![
            vec![
                s(""),
                s("Date"),
                s("Day"),
                s("Consultant"),
                s("Started at"),
                s("Amount of Hours"),
                s("Type"),
                s("Details"),
                s("Parties"),
                s("Togg Division"),
            ],
            vec![
                s(""),
                n(1),
                s("Thursday"),
                s("Kaan"),
                json!(0.375),
                json!(1.5),
                s("Working"),
                s("Eski"),
                s("ADBA"),
                s("Trumore"),
            ],
            vec![s(""), n(2), s("Friday"), s("Kaan")],
            vec![s(""), n(3), s("Saturday"), s("Kaan")],
        ];
        let mut sh = Sheet::new(7, "Mart 2026".into(), grid).unwrap();
        sh.day_formulas = Some(vec![
            String::new(),
            "=SWITCH(WEEKDAY(B2),5,\"Thursday\")".into(),
            "=SWITCH(WEEKDAY(B3),6,\"Friday\")".into(),
            "=SWITCH(WEEKDAY(B4),7,\"Saturday\")".into(),
        ]);
        sh
    }

    fn details(sh: &Sheet) -> Vec<(u32, u32, String)> {
        sh.rows()
            .map(|r| (r.row, r.date.day(), r.details))
            .collect()
    }

    fn kinds(sh: &Sheet) -> Vec<String> {
        sh.requests
            .iter()
            .map(|r| r.as_object().unwrap().keys().next().unwrap().clone())
            .collect()
    }

    #[test]
    fn the_weekday_formula_follows_the_sheets_locale() {
        assert!(semicolon_locale("tr_TR") && semicolon_locale("de") && !semicolon_locale("en_US"));
        // Day sütununda formül yok: haftanın günü formülü yazılır.
        let mut sh = sheet();
        sh.day_formulas = Some(Vec::new());
        sh.grid.iter_mut().skip(1).for_each(|r| r[2] = Value::Null);
        sh.semicolons = true;
        sh.plan_insert("Kaan", &row(5, 9, 1.0, "Yeni"));
        let formula = sh
            .requests
            .iter()
            .find_map(|r| {
                r["updateCells"]["rows"][0]["values"][0]["userEnteredValue"]["formulaValue"]
                    .as_str()
            })
            .unwrap();
        assert!(
            formula.starts_with("=SWITCH(WEEKDAY(B5);1;\"Sunday\""),
            "{formula}"
        );
    }

    #[test]
    fn rows_are_read_like_the_script() {
        let sh = sheet();
        let r = sh.rows().next().unwrap();
        assert_eq!(
            (r.row, r.date, r.start, r.hours, r.consultant.as_str()),
            (2, d(1), NaiveTime::from_hms_opt(9, 0, 0), Some(1.5), "Kaan")
        );
        assert_eq!(sh.rows().count(), 1, "boş gün satırları kayıt değil");
        assert_eq!(letter(2), "B");
        assert_eq!(letter(28), "AB");
        assert_eq!(
            spreadsheet_id("https://docs.google.com/spreadsheets/d/1KoL_-q6Cxq1yqfqCG61rSZHsxHpnsgyQtz_QGIZrR9U/edit?gid=0#gid=0").as_deref(),
            Some("1KoL_-q6Cxq1yqfqCG61rSZHsxHpnsgyQtz_QGIZrR9U")
        );
        assert_eq!(encode("'Mart 2026'"), "%27Mart%202026%27");
    }

    #[test]
    fn append_fills_blank_days_inserts_after_and_marks() {
        let mut sh = sheet();
        let old = Marker {
            meta_id: 5,
            key: "eski".into(),
            fresh: true,
            row: 2,
        };
        let done = sh.plan_append(
            "Kaan",
            &[
                ("id-2".into(), row(2, 9, 1.0, "Bir")),
                ("id-1".into(), row(1, 14, 2.0, "Ek")),
                ("id-3".into(), row(2, 11, 1.0, "İki")),
            ],
            &[old],
        );
        assert_eq!((done.filled, done.inserted, done.skipped), (1, 2, 0));
        assert_eq!(
            details(&sh),
            [
                (2, 1, "Eski".into()),
                (3, 1, "Ek".into()),
                (4, 2, "Bir".into()),
                (5, 2, "İki".into())
            ]
        );
        let k = kinds(&sh);
        assert_eq!(
            k[0], "deleteDeveloperMetadata",
            "önceki aktarımın işaretleri kalkar"
        );
        assert_eq!(k.iter().filter(|k| *k == "insertDimension").count(), 2);
        assert_eq!(
            k.iter().filter(|k| *k == "createDeveloperMetadata").count(),
            3
        );
        // Eklenen satırın Day hücresi üstteki formülden kopyalanır; dolu gün satırınınki korunur.
        assert_eq!(k.iter().filter(|k| *k == "copyPaste").count(), 2);
        // Aynı kayıt ikinci kez yazılmaz.
        let again = sh.plan_append("Kaan", &[("id-2".into(), row(2, 9, 1.0, "Bir"))], &[]);
        assert_eq!(again.skipped, 1);
        // Ortak tabloda iş arkadaşının aynı içerikli satırı bizimki sayılmaz: kayıt yazılır.
        let other = sh.plan_append("Ayşe", &[("id-9".into(), row(2, 9, 1.0, "Bir"))], &[]);
        assert_eq!((other.skipped, other.inserted), (0, 1));
        // Bizimkiyle aynı içerikli iki satır var; danışmanıyla yalnızca bizimki bulunur.
        let mut ours = sh
            .rows()
            .find(|r| r.details == "Bir" && r.consultant == "Kaan")
            .unwrap();
        ours.row = 0;
        let theirs = sh.rows().find(|r| r.consultant == "Ayşe").unwrap();
        assert_ne!(sh.locate(&ours).unwrap(), theirs.row);
    }

    #[test]
    fn rows_are_kept_in_day_and_time_order() {
        let mut sh = sheet();
        // 1 Eki 09:00 "Eski" var; 2 Eki'nin boş satırı 10:00 ile dolar.
        sh.plan_insert("Kaan", &row(2, 10, 1.0, "On"));
        // Önce gelen saat üstüne, arada kalan araya eklenir; hiçbir satır silinmez.
        sh.plan_insert("Kaan", &row(2, 8, 1.0, "Sekiz"));
        sh.plan_insert("Kaan", &row(2, 9, 1.0, "Dokuz"));
        sh.plan_insert("Kaan", &row(1, 8, 0.5, "Erken"));
        sh.plan_insert("Kaan", &row(2, 12, 1.0, "Öğlen"));
        assert_eq!(
            details(&sh),
            [
                (2, 1, "Erken".into()),
                (3, 1, "Eski".into()),
                (4, 2, "Sekiz".into()),
                (5, 2, "Dokuz".into()),
                (6, 2, "On".into()),
                (7, 2, "Öğlen".into()),
            ]
        );
        assert_eq!(sh.date_at(8), Some(d(3)), "3 Eki'nin boş satırı yerinde");
        assert!(
            !kinds(&sh).contains(&"deleteDimension".to_string()),
            "eklemede satır silinmez"
        );
        // Saati değişen kayıt yeni yerine taşınır; sırası bozulmuyorsa yerinde kalır.
        let on = sh.rows().find(|r| r.details == "On").unwrap();
        assert_eq!(
            sh.plan_update("Kaan", &on, &row(2, 7, 1.0, "On")).unwrap(),
            4
        );
        let dokuz = sh.rows().find(|r| r.details == "Dokuz").unwrap();
        sh.requests.clear();
        let r = sh
            .plan_update("Kaan", &dokuz, &row(2, 11, 1.0, "Dokuz"))
            .unwrap();
        assert_eq!(r, dokuz.row);
        assert!(!kinds(&sh).contains(&"deleteDimension".to_string()));
        assert_eq!(
            details(&sh)
                .into_iter()
                .filter(|x| x.1 == 2)
                .map(|x| x.2)
                .collect::<Vec<_>>(),
            ["On", "Sekiz", "Dokuz", "Öğlen"]
        );
    }

    #[test]
    fn update_moves_between_days_and_remove_and_undo_restore_the_layout() {
        let mut sh = sheet();
        sh.plan_append(
            "Kaan",
            &[
                ("a".into(), row(2, 9, 1.0, "Bir")),
                ("b".into(), row(2, 11, 1.0, "İki")),
            ],
            &[],
        );
        let iki = sh.rows().find(|r| r.details == "İki").unwrap();
        // Satır numarası kaymış ipucu: içerikten bulunur; 3 Eki'nin boş satırına taşınır.
        let mut hint = iki.clone();
        hint.row = 2;
        sh.requests.clear();
        let r = sh
            .plan_update("Kaan", &hint, &row(3, 11, 1.0, "İki"))
            .unwrap();
        assert_eq!(
            details(&sh),
            [
                (2, 1, "Eski".into()),
                (3, 2, "Bir".into()),
                (4, 3, "İki".into())
            ]
        );
        assert_eq!(r, 4);
        assert_eq!(
            kinds(&sh)[0],
            "deleteDimension",
            "2 Eki'nin ikinci satırı silinir"
        );
        // Eski içerikle: değişmiş.
        assert!(matches!(
            sh.plan_update("", &iki, &row(3, 11, 1.0, "x")),
            Err(Error::Changed)
        ));
        // Günün tek satırı kaldırılınca gün satırı kalır, işareti kalkar.
        let bir = sh.rows().find(|r| r.details == "Bir").unwrap();
        sh.requests.clear();
        let marker = Marker {
            meta_id: 9,
            key: "a".into(),
            fresh: false,
            row: bir.row,
        };
        sh.plan_remove(&bir, &[marker]).unwrap();
        assert_eq!(details(&sh), [(2, 1, "Eski".into()), (4, 3, "İki".into())]);
        assert_eq!(sh.date_at(3), Some(d(2)));
        assert!(kinds(&sh).contains(&"deleteDeveloperMetadata".to_string()));
        // Geri ekleme gününün boş satırına.
        assert_eq!(sh.plan_insert("Kaan", &row(2, 9, 1.0, "Bir")), 3);
        // Aktarımı geri alma: eklenen satır silinir, doldurulan boşaltılır.
        let undone = sh.plan_undo(
            &["a-uzun-kimlik".into(), "b".into(), "yok".into()],
            &[
                Marker {
                    meta_id: 1,
                    key: "a-uzun-kimlik".into(),
                    fresh: false,
                    row: 3,
                },
                Marker {
                    meta_id: 2,
                    key: "b".into(),
                    fresh: true,
                    row: 4,
                },
            ],
        );
        assert_eq!((undone.removed, undone.cleared, undone.missing), (1, 1, 1));
        // Bulunamayan kayıt geri alınmış sayılmaz (yeniden aktarılmamış işaretlenmez).
        assert_eq!(undone.undone, ["a-uzun-kimlik", "b"]);
        assert_eq!(
            details(&sh),
            [(2, 1, "Eski".into())],
            "2 Eki boşaltıldı, 3 Eki'nin eklenen satırı silindi"
        );
    }
}
