//! Zaman çizelgesi kayıtlarını kullanıcının kendi Excel dosyasına ekler.
//!
//! Dosyanın düzeni başlık satırındaki sütun adlarından bulunur ("Date", "Day",
//! "Consultant", "Started at", "Amount of Hours", "Type", "Details", "Parties",
//! "… Division"); sütunların yeri değişse de çalışır. Her kayıt için önce o günün
//! önceden doldurulmuş boş satırı (tarih ve danışman yazılı, ayrıntı boş) kullanılır;
//! yetmezse gün ve saat sırasını bozmayacak yere (kendisinden önce gelen kaydın altına)
//! satır eklenir ([`slot`]). Eklenen satırlar biçimini üstteki satırdan alır. Yazmadan önce
//! dosyanın yanına zaman damgalı yedek alınır.
//!
//! Dosyadaki kayıtlar okunur ([`list`]) ve tek tek değiştirilir ya da kaldırılır ([`update`],
//! [`remove`]): satır, beklenen eski içeriğiyle bulunur; dosyada değişmişse dokunulmaz.
//!
//! Google Sheets'e aynı kurallarla yazmak için [`sheets`], aylık müşteri raporu için [`report`].

pub mod gsheets;
pub mod report;
pub mod sheets;

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, NaiveTime, Timelike};
use umya_spreadsheet::{Spreadsheet, Worksheet};

/// Excel'e yazılacak bir satır.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub hours: f64,
    pub kind: String,
    pub details: String,
    pub party: String,
    pub division: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Excel dosyası okunamadı: {0}")]
    Read(String),
    #[error("Excel dosyası yazılamadı (Excel'de açıksa kapatıp tekrar dene): {0}")]
    Write(String),
    #[error("yedek alınamadı: {0}")]
    Backup(std::io::Error),
    #[error("dosyada sayfa yok")]
    NoSheet,
    #[error("başlık satırında \"{0}\" sütunu bulunamadı")]
    MissingColumn(&'static str),
    #[error("Google Sheets: {0}")]
    Sheets(String),
    #[error("satır tabloda değişmiş ya da silinmiş; sayfa yenilendi, tekrar dene")]
    Changed,
}

/// Dosyada bulunan bir kayıt satırı (Kum'un yazdığı ya da elle girilmiş).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRow {
    /// Satır numarası (1'den başlar; başlık 1. satır). Yazarken yalnızca ipucudur: satır
    /// kaymışsa içeriğinden bulunur.
    pub row: u32,
    pub date: NaiveDate,
    /// Başlangıç; hücre boşsa ya da saat değilse `None`.
    pub start: Option<NaiveTime>,
    pub hours: Option<f64>,
    pub kind: String,
    pub details: String,
    pub party: String,
    pub division: String,
    pub consultant: String,
}

impl SheetRow {
    /// İki satırın içeriği aynı mı (satır numarası hariç; metinler kırpılır, birim büyük/küçük
    /// harfe bakmaz, başlangıç dakikaya kadar). Danışman ikisinde de yazılıysa aynı olmalı: ortak
    /// tabloda bir iş arkadaşının aynı içerikli satırı bizimki sayılmasın.
    pub fn same(&self, other: &SheetRow) -> bool {
        let minute = |t: Option<NaiveTime>| t.map(|t| (t.hour(), t.minute()));
        let hours = match (self.hours, other.hours) {
            (Some(a), Some(b)) => (a - b).abs() < 1e-6,
            (a, b) => a.is_none() && b.is_none(),
        };
        self.date == other.date
            && minute(self.start) == minute(other.start)
            && hours
            && self.kind.trim() == other.kind.trim()
            && self.details.trim() == other.details.trim()
            && self.party.trim() == other.party.trim()
            && self
                .division
                .trim()
                .eq_ignore_ascii_case(other.division.trim())
            && same_consultant(&self.consultant, &other.consultant)
    }
}

/// Danışmanlar çelişmiyor: biri boşsa (sütun yok ya da yazılmamış) ya da aynıysa.
fn same_consultant(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    a.is_empty() || b.is_empty() || a == b
}

pub type Result<T> = std::result::Result<T, Error>;

/// Eklemenin özeti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    /// Önceden doldurulmuş boş satıra yazılan kayıt sayısı.
    pub filled: usize,
    /// Yeni satır eklenerek yazılan kayıt sayısı.
    pub inserted: usize,
    pub backup: PathBuf,
}

/// Başlık satırındaki sütun konumları (1'den başlar).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Columns {
    pub(crate) date: u32,
    pub(crate) day: Option<u32>,
    pub(crate) consultant: Option<u32>,
    pub(crate) start: u32,
    pub(crate) hours: u32,
    pub(crate) kind: u32,
    pub(crate) details: u32,
    pub(crate) party: u32,
    pub(crate) division: u32,
}

pub(crate) const HEADER_ROW: u32 = 1;
pub(crate) const DAY_FORMULA: &str = r#"SWITCH(WEEKDAY(B{r}),1,"Sunday",2,"Monday",3,"Tuesday",4,"Wednesday",5,"Thursday",6,"Friday",7,"Saturday")"#;

fn columns(ws: &Worksheet) -> Result<Columns> {
    let last = ws.get_highest_column().max(1);
    let headers: Vec<String> = (1..=last).map(|c| ws.get_value((c, HEADER_ROW))).collect();
    columns_of(&headers)
}

/// Başlık satırının hücrelerinden (soldan sağa) sütun konumları.
pub(crate) fn columns_of(headers: &[String]) -> Result<Columns> {
    let headers: Vec<(u32, String)> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            (
                i as u32 + 1,
                h.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase(),
            )
        })
        .collect();
    let find = |name: &'static str, pred: &dyn Fn(&str) -> bool| {
        headers
            .iter()
            .find(|(_, h)| pred(h))
            .map(|(c, _)| *c)
            .ok_or(Error::MissingColumn(name))
    };
    Ok(Columns {
        date: find("Date", &|h| h == "date")?,
        day: find("Day", &|h| h == "day").ok(),
        consultant: find("Consultant", &|h| h == "consultant").ok(),
        start: find("Started at", &|h| h.starts_with("started"))?,
        hours: find("Amount of Hours", &|h| h.contains("hours"))?,
        kind: find("Type", &|h| h == "type")?,
        details: find("Details", &|h| h == "details")?,
        party: find("Parties", &|h| h.starts_with("part"))?,
        division: find("Division", &|h| {
            h.contains("division") || h == "project" || h == "proje"
        })?,
    })
}

/// Değerler en sıktan seyreğe (boşlar atlanır).
pub(crate) fn ranked(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for v in values {
        let v = v.trim().to_string();
        if !v.is_empty() {
            *counts.entry(v).or_default() += 1;
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(k, _)| k).collect()
}

/// Birim sütununun başlığından firma: "Togg Division" → "Togg".
pub(crate) fn company_of(header: &str) -> Option<String> {
    let company = header
        .split_whitespace()
        .take_while(|w| !w.eq_ignore_ascii_case("division"))
        .collect::<Vec<_>>()
        .join(" ");
    (!company.is_empty()).then_some(company)
}

/// Excel tarih seri numarası (1900 sistemi) → tarih.
pub(crate) fn serial_to_date(v: &str) -> Option<NaiveDate> {
    let n: f64 = v.trim().parse().ok()?;
    if !(1.0..2_958_466.0).contains(&n) {
        return None;
    }
    NaiveDate::from_ymd_opt(1899, 12, 30)?.checked_add_days(chrono::Days::new(n.floor() as u64))
}

pub(crate) fn date_to_serial(d: NaiveDate) -> f64 {
    (d - NaiveDate::from_ymd_opt(1899, 12, 30).expect("sabit tarih")).num_days() as f64
}

pub(crate) fn time_to_fraction(t: NaiveTime) -> f64 {
    f64::from(t.num_seconds_from_midnight()) / 86_400.0
}

/// Şablondan çıkarılan bilgiler (içe aktarma için).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Template {
    /// Birim sütununun başlığından firma ("Togg Division" → "Togg").
    pub company: Option<String>,
    /// En sık danışman adı.
    pub consultant: Option<String>,
    /// Taraflar, en sık kullanılandan.
    pub parties: Vec<String>,
    /// Birimler (projeler), en sık kullanılandan.
    pub divisions: Vec<String>,
    /// Geçmiş açıklamalar, en sık kullanılandan (otomatik tamamlama).
    pub details: Vec<String>,
}

/// Dosyanın düzenini doğrular ve geçmiş kayıtlardan şablon bilgilerini çıkarır.
pub fn inspect(path: &Path) -> Result<Template> {
    let book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    let ws = book.get_sheet(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let ranked = |col: u32| {
        ranked(((HEADER_ROW + 1)..=ws.get_highest_row()).map(|r| ws.get_value((col, r))))
    };
    Ok(Template {
        company: company_of(&ws.get_value((cols.division, HEADER_ROW))),
        consultant: cols.consultant.and_then(|c| ranked(c).into_iter().next()),
        parties: ranked(cols.party),
        divisions: ranked(cols.division),
        details: ranked(cols.details),
    })
}

/// `rows` kayıtlarını `path` dosyasının ilk sayfasına ekler.
pub fn append(path: &Path, consultant: &str, rows: &[Row]) -> Result<Appended> {
    let mut book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    // Önce bellekte uygula (sütun eksikse dosyaya hiç dokunulmaz), sonra yedekle ve yaz.
    let (filled, inserted) = write_rows(&mut book, consultant, rows)?;
    let backup = backup_path(path);
    std::fs::copy(path, &backup).map_err(Error::Backup)?;
    umya_spreadsheet::writer::xlsx::write(&book, path).map_err(|e| Error::Write(e.to_string()))?;
    Ok(Appended {
        filled,
        inserted,
        backup,
    })
}

fn backup_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("zaman-cizelgesi");
    let name = format!("{stem}.yedek-{}.xlsx", Local::now().format("%Y%m%d-%H%M%S"));
    path.with_file_name(name)
}

/// Tek satır düzenlemelerinin yedeği: günde bir kez (günün ilk düzenlemesinden önceki hali);
/// her hücre değişikliği ayrı yedek bırakmasın.
fn edit_backup(path: &Path) -> Result<()> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("zaman-cizelgesi");
    let backup = path.with_file_name(format!(
        "{stem}.yedek-{}.xlsx",
        Local::now().format("%Y%m%d")
    ));
    if !backup.exists() {
        std::fs::copy(path, &backup).map_err(Error::Backup)?;
    }
    Ok(())
}

/// Başlangıç hücresi → saat: gün kesri (tarihli de olabilir) ya da "09:30" metni.
pub(crate) fn cell_time(v: &str) -> Option<NaiveTime> {
    let v = v.trim();
    if let Ok(n) = v.parse::<f64>() {
        if n < 0.0 {
            return None;
        }
        let secs = ((n.fract() * 86_400.0).round() as u32).min(86_399);
        return NaiveTime::from_num_seconds_from_midnight_opt(secs, 0);
    }
    let (h, rest) = v.split_once(':')?;
    let m: String = rest.chars().take_while(char::is_ascii_digit).collect();
    NaiveTime::from_hms_opt(h.trim().parse().ok()?, m.parse().ok()?, 0)
}

/// Saat hücresi → sayı ("1,5" de olur).
pub(crate) fn cell_hours(v: &str) -> Option<f64> {
    v.trim().replace(',', ".").parse().ok()
}

fn read_row(ws: &Worksheet, cols: Columns, r: u32) -> Option<SheetRow> {
    let date = serial_to_date(&ws.get_value((cols.date, r)))?;
    let text = |c: u32| ws.get_value((c, r)).trim().to_string();
    let row = SheetRow {
        row: r,
        date,
        start: cell_time(&ws.get_value((cols.start, r))),
        hours: cell_hours(&ws.get_value((cols.hours, r))),
        kind: text(cols.kind),
        details: text(cols.details),
        party: text(cols.party),
        division: text(cols.division),
        consultant: cols.consultant.map(text).unwrap_or_default(),
    };
    // Önceden doldurulmuş boş gün satırı kayıt değildir.
    let blank = row.kind.is_empty() && row.details.is_empty() && row.hours.is_none();
    (!blank).then_some(row)
}

/// Başlık altındaki bir satır, yerleştirme için ([`slot`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Line {
    /// Tarihsiz satır (boşluk, toplam…): sıralamada yok sayılır.
    Undated,
    /// Önceden doldurulmuş boş gün satırı.
    Blank(NaiveDate),
    /// Kayıt; başlangıcı boşsa günün başında sayılır.
    Entry(NaiveDate, Option<NaiveTime>),
}

fn key(date: NaiveDate, start: Option<NaiveTime>) -> (NaiveDate, NaiveTime) {
    (date, start.unwrap_or(NaiveTime::MIN))
}

/// `date` günü `start` saatli yeni kaydın satırı: tablo gün ve saate göre sıralı kalır. Gün ve
/// saati kendisinden önce gelen son kaydın altındaki aralıkta (bir sonraki kayda kadar) o günün
/// boş satırı varsa o doldurulur; yoksa aralıkta tarihi önce gelen boş gün satırlarının altına
/// satır eklenir. Hiçbir satır silinmez. `lines[0]` başlığın altındaki satırdır.
/// (satır, eklenecek mi)
pub(crate) fn slot(lines: &[Line], date: NaiveDate, start: NaiveTime) -> (u32, bool) {
    let new = (date, start);
    let row = |i: usize| HEADER_ROW + 1 + i as u32;
    let prev = lines
        .iter()
        .rposition(|l| matches!(*l, Line::Entry(d, s) if key(d, s) <= new));
    let from = prev.map_or(0, |i| i + 1);
    let to = lines[from..]
        .iter()
        .position(|l| matches!(l, Line::Entry(..)))
        .map_or(lines.len(), |i| from + i);
    if let Some(i) = (from..to).find(|&i| lines[i] == Line::Blank(date)) {
        return (row(i), false);
    }
    let after = (from..to)
        .rev()
        .find(|&i| matches!(lines[i], Line::Blank(d) if d <= date))
        .or(prev);
    (after.map_or(HEADER_ROW + 1, |i| row(i) + 1), true)
}

/// `lines` içindeki `i` kaydı `date`/`start` ile yerinde kalabilir mi: üstündeki kayıt önce,
/// altındaki sonra geliyor.
pub(crate) fn in_order(lines: &[Line], i: usize, date: NaiveDate, start: NaiveTime) -> bool {
    let new = key(date, Some(start));
    let entry = |l: &Line| match *l {
        Line::Entry(d, s) => Some(key(d, s)),
        _ => None,
    };
    lines[..i]
        .iter()
        .rev()
        .find_map(entry)
        .is_none_or(|k| k <= new)
        && lines[i + 1..]
            .iter()
            .find_map(entry)
            .is_none_or(|k| k >= new)
}

/// `from`–`to` (dahil) tarihli kayıt satırları, dosyadaki sırayla.
pub fn list(path: &Path, from: NaiveDate, to: NaiveDate) -> Result<Vec<SheetRow>> {
    let book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    let ws = book.get_sheet(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    Ok(((HEADER_ROW + 1)..=ws.get_highest_row())
        .filter_map(|r| read_row(ws, cols, r))
        .filter(|s| (from..=to).contains(&s.date))
        .collect())
}

/// `expect` satırının bugünkü yeri: ipucundaki satır hâlâ aynıysa o, değilse içeriği aynı tek
/// satır. Bulunamazsa (ya da birden çoksa) dosyaya dokunulmaz.
fn locate(ws: &Worksheet, cols: Columns, expect: &SheetRow) -> Result<u32> {
    let at = |r: u32| read_row(ws, cols, r).is_some_and(|s| s.same(expect));
    if expect.row > HEADER_ROW && at(expect.row) {
        return Ok(expect.row);
    }
    let found: Vec<u32> = ((HEADER_ROW + 1)..=ws.get_highest_row())
        .filter(|&r| at(r))
        .collect();
    match found[..] {
        [r] => Ok(r),
        _ => Err(Error::Changed),
    }
}

/// `expect` satırını `row` değerleriyle değiştirir; yazılan satırın numarası. Tarih değiştiyse
/// ya da yeni saatiyle yerinde sıra bozulacaksa satır eski yerinden kaldırılır ([`vacate`]) ve
/// Kum'un aktarımıyla aynı kurallarla yerleşir ([`place`]): gün ve saat sırası bozulmaz.
pub fn update(path: &Path, consultant: &str, expect: &SheetRow, row: &Row) -> Result<u32> {
    let mut book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let mut r = locate(ws, cols, expect)?;
    let moved = row.date != expect.date
        || !in_order(
            &lines(ws, cols),
            (r - HEADER_ROW - 1) as usize,
            row.date,
            row.start,
        );
    if moved {
        vacate(ws, cols, r, expect.date);
        r = place(ws, cols, row.date, row.start).0;
    }
    put_row(ws, r, cols, consultant, row);
    edit_backup(path)?;
    umya_spreadsheet::writer::xlsx::write(&book, path).map_err(|e| Error::Write(e.to_string()))?;
    Ok(r)
}

/// `expect` satırını kaldırır: günün başka satırı varsa satır silinir, yoksa günün satırı
/// (tarih, gün, danışman) kalır ve kayıt hücreleri boşaltılır.
pub fn remove(path: &Path, expect: &SheetRow) -> Result<()> {
    let mut book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let r = locate(ws, cols, expect)?;
    vacate(ws, cols, r, expect.date);
    edit_backup(path)?;
    umya_spreadsheet::writer::xlsx::write(&book, path).map_err(|e| Error::Write(e.to_string()))?;
    Ok(())
}

/// Tek satırı gününe ekler (silmenin geri alınması); yazılan satırın numarası.
pub fn insert(path: &Path, consultant: &str, row: &Row) -> Result<u32> {
    let mut book =
        umya_spreadsheet::reader::xlsx::read(path).map_err(|e| Error::Read(e.to_string()))?;
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let r = place(ws, cols, row.date, row.start).0;
    put_row(ws, r, cols, consultant, row);
    edit_backup(path)?;
    umya_spreadsheet::writer::xlsx::write(&book, path).map_err(|e| Error::Write(e.to_string()))?;
    Ok(r)
}

/// `r` satırındaki `date` günlü kaydı kaldırır: günün başka satırı varsa satır silinir, yoksa
/// gün satırı kalır, kayıt hücreleri boşaltılır.
fn vacate(ws: &mut Worksheet, cols: Columns, r: u32, date: NaiveDate) {
    let others = ((HEADER_ROW + 1)..=ws.get_highest_row())
        .any(|x| x != r && serial_to_date(&ws.get_value((cols.date, x))) == Some(date));
    if others {
        ws.remove_row(&r, &1);
    } else {
        for c in [
            cols.start,
            cols.hours,
            cols.kind,
            cols.details,
            cols.party,
            cols.division,
        ] {
            ws.get_cell_mut((c, r)).set_value("");
        }
    }
}

fn lines(ws: &Worksheet, cols: Columns) -> Vec<Line> {
    ((HEADER_ROW + 1)..=ws.get_highest_row())
        .map(|r| match serial_to_date(&ws.get_value((cols.date, r))) {
            None => Line::Undated,
            Some(d) => read_row(ws, cols, r).map_or(Line::Blank(d), |e| Line::Entry(d, e.start)),
        })
        .collect()
}

/// `date` günü `start` saatli kaydın satırı ([`slot`]); eklenen satır biçimini üstteki kayıt
/// satırından (başlığın hemen altındaysa alttakinden) alır. (satır, eklendi mi)
fn place(ws: &mut Worksheet, cols: Columns, date: NaiveDate, start: NaiveTime) -> (u32, bool) {
    let (r, fresh) = slot(&lines(ws, cols), date, start);
    if fresh {
        ws.insert_new_row(&r, &1);
        let from = if r - 1 > HEADER_ROW { r - 1 } else { r + 1 };
        if from <= ws.get_highest_row() {
            copy_row_style(ws, from, r, cols);
        }
    }
    (r, fresh)
}

fn write_rows(book: &mut Spreadsheet, consultant: &str, rows: &[Row]) -> Result<(usize, usize)> {
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| (r.date, r.start));
    let (mut filled, mut inserted) = (0, 0);
    for row in sorted {
        let (target, fresh) = place(ws, cols, row.date, row.start);
        if fresh {
            inserted += 1;
        } else {
            filled += 1;
        }
        put_row(ws, target, cols, consultant, row);
    }
    Ok((filled, inserted))
}

fn copy_row_style(ws: &mut Worksheet, from: u32, to: u32, cols: Columns) {
    if from == HEADER_ROW || from == to {
        return;
    }
    let last = ws.get_highest_column().max(cols.division);
    for c in 1..=last {
        let style = ws.get_style((c, from)).clone();
        ws.set_style((c, to), style);
    }
}

fn put_row(ws: &mut Worksheet, r: u32, cols: Columns, consultant: &str, row: &Row) {
    let date = ws.get_cell_mut((cols.date, r));
    date.set_value_number(date_to_serial(row.date));
    ws.get_style_mut((cols.date, r))
        .get_number_format_mut()
        .set_format_code("d/m/yy");
    if let Some(day) = cols.day {
        // Yalnızca boşsa: önceden doldurulmuş satırın kendi formülü korunur.
        if ws.get_value((day, r)).trim().is_empty()
            && ws
                .get_cell((day, r))
                .is_none_or(|c| c.get_formula().is_empty())
        {
            let letter = umya_spreadsheet::helper::coordinate::string_from_column_index(&cols.date);
            let formula = DAY_FORMULA.replace("B{r}", &format!("{letter}{r}"));
            ws.get_cell_mut((day, r)).set_formula(formula);
        }
    }
    if let Some(c) = cols.consultant
        && !consultant.trim().is_empty()
    {
        put_text(ws, c, r, consultant.trim());
    }
    ws.get_cell_mut((cols.start, r))
        .set_value_number(time_to_fraction(row.start));
    ws.get_style_mut((cols.start, r))
        .get_number_format_mut()
        .set_format_code("hh:mm");
    // Genel biçim: 1, 0,5, 0,25 (gereksiz sıfırlar olmadan).
    ws.get_cell_mut((cols.hours, r)).set_value_number(row.hours);
    ws.get_style_mut((cols.hours, r))
        .get_number_format_mut()
        .set_format_code("General");
    put_text(ws, cols.kind, r, &row.kind);
    put_text(ws, cols.details, r, &row.details);
    put_text(ws, cols.party, r, &row.party);
    put_text(ws, cols.division, r, &row.division);
}

/// Metin hücresi metin olarak yazılır: `set_value` "123", "TRUE", "#N/A" gibi açıklamaları sayı,
/// mantıksal değer ya da hata yapardı. Boş metin hücreyi boşaltır.
fn put_text(ws: &mut Worksheet, c: u32, r: u32, s: &str) {
    let cell = ws.get_cell_mut((c, r));
    if s.is_empty() {
        cell.set_value("");
    } else {
        cell.set_value_string(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    /// Şablonun düzeni: başlık, dolu bir satır, ileri günler için önceden doldurulmuş satırlar.
    fn template(path: &Path) {
        let mut book = umya_spreadsheet::new_file();
        let ws = book.get_sheet_mut(&0).unwrap();
        let headers = [
            " ",
            "Date",
            "Day",
            "Consultant",
            "Started at",
            "Amount of\n Hours",
            "Type",
            "Details",
            "Parties",
            "Togg Division",
        ];
        for (i, h) in headers.iter().enumerate() {
            ws.get_cell_mut((i as u32 + 1, 1)).set_value(*h);
        }
        ws.get_style_mut((8, 2)).get_font_mut().set_bold(true);
        // 2: 1 Eki dolu kayıt; 3–5: 2, 3, 4 Eki boş (tarih + gün formülü + danışman).
        for (r, day) in [(2u32, 1u32), (3, 2), (4, 3), (5, 4)] {
            ws.get_cell_mut((2, r))
                .set_value_number(date_to_serial(d(day)));
            ws.get_cell_mut((3, r))
                .set_formula(DAY_FORMULA.replace("{r}", &r.to_string()));
            ws.get_cell_mut((4, r)).set_value("Kaan Baytur");
        }
        ws.get_cell_mut((7, 2)).set_value("Working");
        ws.get_cell_mut((8, 2)).set_value("Eski kayıt");
        umya_spreadsheet::writer::xlsx::write(&book, path).unwrap();
    }

    fn row(day: u32, hh: u32, hours: f64, details: &str) -> Row {
        Row {
            date: d(day),
            start: NaiveTime::from_hms_opt(hh, 30, 0).unwrap(),
            hours,
            kind: "Working".into(),
            details: details.into(),
            party: "ADBA".into(),
            division: "Trumore".into(),
        }
    }

    #[test]
    fn slots_keep_day_and_time_order() {
        let t = |h: u32| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let lines = [
            Line::Entry(d(1), None),
            Line::Entry(d(1), Some(t(10))),
            Line::Undated,
            Line::Blank(d(2)),
            Line::Entry(d(3), Some(t(9))),
            Line::Blank(d(5)),
        ];
        // Satır numaraları: başlık 1, lines[0] 2. Saatsiz kayıt günün başında sayılır.
        assert_eq!(slot(&lines, d(1), t(9)), (3, true), "10:00'ın üstü");
        assert_eq!(
            slot(&lines, d(1), t(11)),
            (4, true),
            "günün son kaydının altı"
        );
        assert_eq!(slot(&lines, d(2), t(15)), (5, false), "günün boş satırı");
        assert_eq!(
            slot(&lines, d(3), t(8)),
            (6, true),
            "2 Eki'nin altı, 3 Eki 09:00'ın üstü"
        );
        assert_eq!(slot(&lines, d(4), t(9)), (7, true));
        assert_eq!(slot(&lines, d(6), t(9)), (8, true));
        assert_eq!(slot(&[], d(1), t(9)), (2, true));
        assert!(in_order(&lines, 4, d(3), t(7)));
        assert!(!in_order(&lines, 0, d(1), t(23)));
    }

    #[test]
    fn fills_prefilled_rows_then_inserts_and_keeps_order() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        let rows = [
            row(3, 10, 1.0 + 10.0 / 60.0, "İkinci"),
            row(2, 9, 2.5, "Tek"),
            row(3, 9, 0.75, "Birinci"),
            row(1, 14, 1.0, "Bir Ekim ek"),
        ];
        let done = append(&path, "Kaan Baytur", &rows).unwrap();
        assert_eq!((done.filled, done.inserted), (2, 2));
        assert!(done.backup.exists());

        let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
        let ws = book.get_sheet(&0).unwrap();
        let line = |r: u32| -> (Option<NaiveDate>, String, String) {
            (
                serial_to_date(&ws.get_value((2, r))),
                ws.get_value((8, r)),
                ws.get_value((7, r)),
            )
        };
        // 1 Eki: eski kayıt + altına eklenen; 2 Eki: boş satıra; 3 Eki: boş satır + eklenen; 4 Eki boş kalır.
        assert_eq!(line(2), (Some(d(1)), "Eski kayıt".into(), "Working".into()));
        assert_eq!(
            line(3),
            (Some(d(1)), "Bir Ekim ek".into(), "Working".into())
        );
        assert_eq!(line(4), (Some(d(2)), "Tek".into(), "Working".into()));
        assert_eq!(line(5).1, "Birinci");
        assert_eq!((line(6).0, line(6).1.as_str()), (Some(d(3)), "İkinci"));
        assert_eq!((line(7).0, line(7).1.as_str()), (Some(d(4)), ""));
        // Saat tam değer, başlangıç hh:mm, gün formülü kendi satırına başvurur.
        let hours: f64 = ws.get_value((6, 6)).parse().unwrap();
        assert!((hours - (1.0 + 10.0 / 60.0)).abs() < 1e-9);
        assert_eq!(
            ws.get_style((5, 6))
                .get_number_format()
                .unwrap()
                .get_format_code(),
            "hh:mm"
        );
        assert!(ws.get_cell((3, 6)).unwrap().get_formula().contains("B6"));
        assert_eq!(ws.get_value((4, 3)), "Kaan Baytur");
        assert_eq!(ws.get_value((9, 6)), "ADBA");
        assert_eq!(ws.get_value((10, 6)), "Trumore");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_earlier_time_is_inserted_above_in_the_file() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        assert_eq!(
            insert(&path, "Kaan Baytur", &row(3, 10, 1.0, "On")).unwrap(),
            4
        );
        assert_eq!(
            insert(&path, "Kaan Baytur", &row(3, 9, 1.0, "Dokuz")).unwrap(),
            4
        );
        assert_eq!(
            insert(&path, "Kaan Baytur", &row(3, 11, 1.0, "Onbir")).unwrap(),
            6
        );
        let got: Vec<(u32, u32, String)> = list(&path, d(1), d(9))
            .unwrap()
            .into_iter()
            .map(|r| (r.row, r.date.day(), r.details))
            .collect();
        assert_eq!(
            got,
            [
                (2, 1, "Eski kayıt".into()),
                (4, 3, "Dokuz".into()),
                (5, 3, "On".into()),
                (6, 3, "Onbir".into())
            ]
        );
        // Boş gün satırları yerinde: 2 Eki üstte, 4 Eki altta.
        let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
        let ws = book.get_sheet(&0).unwrap();
        assert_eq!(serial_to_date(&ws.get_value((2, 3))), Some(d(2)));
        assert_eq!(serial_to_date(&ws.get_value((2, 7))), Some(d(4)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn inspects_template() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        append(
            &path,
            "Kaan Baytur",
            &[row(2, 9, 1.0, "PDaI UI/UX"), row(3, 9, 1.0, "PDaI UI/UX")],
        )
        .unwrap();
        let t = inspect(&path).unwrap();
        assert_eq!(t.company.as_deref(), Some("Togg"));
        assert_eq!(t.consultant.as_deref(), Some("Kaan Baytur"));
        assert_eq!(t.parties, ["ADBA"]);
        assert_eq!(t.divisions, ["Trumore"]);
        assert_eq!(t.details, ["PDaI UI/UX", "Eski kayıt"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn lists_updates_and_removes_rows() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        append(
            &path,
            "Kaan Baytur",
            &[
                row(2, 9, 1.5, "Tek"),
                row(3, 9, 1.0, "Bir"),
                row(3, 11, 2.0, "İki"),
            ],
        )
        .unwrap();
        // 1 Eki'nin eski kaydı (saat ve başlangıç yok) da gelir; 4 Eki'nin boş satırı gelmez.
        let all = list(&path, d(1), d(4)).unwrap();
        let short: Vec<_> = all
            .iter()
            .map(|s| {
                (
                    s.row,
                    s.date.day(),
                    s.start.map(|t| t.hour()),
                    s.hours,
                    s.details.as_str(),
                )
            })
            .collect();
        assert_eq!(
            short,
            [
                (2, 1, None, None, "Eski kayıt"),
                (3, 2, Some(9), Some(1.5), "Tek"),
                (4, 3, Some(9), Some(1.0), "Bir"),
                (5, 3, Some(11), Some(2.0), "İki"),
            ]
        );
        assert_eq!(all[1].consultant, "Kaan Baytur");
        assert_eq!(all[1].start, NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(list(&path, d(3), d(3)).unwrap().len(), 2);

        // Satır numarası kaymış ipucu: içerikten bulunur.
        let mut expect = all[3].clone();
        expect.row = 2;
        let r = update(
            &path,
            "Kaan Baytur",
            &expect,
            &row(3, 11, 2.25, "İki, düzeltildi"),
        )
        .unwrap();
        assert_eq!(r, 5);
        let now = list(&path, d(3), d(3)).unwrap();
        assert_eq!(
            (now[1].hours, now[1].details.as_str()),
            (Some(2.25), "İki, düzeltildi")
        );
        // Eski içerikle ikinci kez: satır değişmiş.
        assert!(matches!(
            update(&path, "", &all[3], &row(3, 11, 1.0, "x")),
            Err(Error::Changed)
        ));

        // Günün başka satırı varsa satır silinir; tek satırsa gün satırı boş kalır.
        remove(&path, &now[0]).unwrap();
        remove(&path, &all[1]).unwrap();
        let left = list(&path, d(1), d(4)).unwrap();
        let details: Vec<_> = left.iter().map(|s| s.details.as_str()).collect();
        assert_eq!(details, ["Eski kayıt", "İki, düzeltildi"]);
        let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
        let ws = book.get_sheet(&0).unwrap();
        assert_eq!(
            serial_to_date(&ws.get_value((2, 3))),
            Some(d(2)),
            "2 Eki satırı kalır"
        );
        assert_eq!(ws.get_value((4, 3)), "Kaan Baytur");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn moving_a_row_to_another_day_and_restoring_a_removed_one() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        append(
            &path,
            "Kaan Baytur",
            &[row(3, 9, 1.0, "Bir"), row(3, 11, 2.0, "İki")],
        )
        .unwrap();
        let day = |n: u32| -> Vec<String> {
            list(&path, d(n), d(n))
                .unwrap()
                .into_iter()
                .map(|s| s.details)
                .collect()
        };
        let iki = list(&path, d(3), d(3)).unwrap()[1].clone();
        // 3 Eki → 4 Eki: 4 Eki'nin boş satırına yazılır, 3 Eki'de satır silinir.
        let r = update(&path, "Kaan Baytur", &iki, &row(4, 11, 2.0, "İki")).unwrap();
        assert_eq!(day(3), ["Bir"]);
        assert_eq!(day(4), ["İki"]);
        // 1 Eki'ye (dolu gün): o günün altına eklenir, tarih sırası korunur.
        let moved = list(&path, d(4), d(4)).unwrap().remove(0);
        assert_eq!(moved.row, r);
        update(&path, "Kaan Baytur", &moved, &row(1, 15, 2.0, "İki")).unwrap();
        assert_eq!(day(1), ["Eski kayıt", "İki"]);
        assert!(day(4).is_empty(), "4 Eki'nin gün satırı boş kalır");
        let dates: Vec<NaiveDate> = list(&path, d(1), d(4))
            .unwrap()
            .iter()
            .map(|s| s.date)
            .collect();
        assert!(dates.is_sorted(), "{dates:?}");
        // Silinen satır geri eklenir.
        let bir = list(&path, d(3), d(3)).unwrap().remove(0);
        remove(&path, &bir).unwrap();
        assert!(day(3).is_empty());
        insert(&path, "Kaan Baytur", &row(3, 9, 1.0, "Bir")).unwrap();
        assert_eq!(day(3), ["Bir"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn text_cells_are_written_as_text() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sablon.xlsx");
        template(&path);
        let mut r = row(2, 9, 1.0, "123");
        r.party = "TRUE".into();
        r.division = "#N/A".into();
        append(&path, "Kaan Baytur", &[r]).unwrap();
        let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
        let ws = book.get_sheet(&0).unwrap();
        for (c, want) in [(8, "123"), (9, "TRUE"), (10, "#N/A"), (4, "Kaan Baytur")] {
            let cell = ws.get_cell((c, 3)).unwrap();
            assert_eq!(
                (cell.get_value().as_ref(), cell.get_data_type()),
                (want, "s")
            );
        }
        // Sayı ve tarih hücreleri sayı kalır.
        assert_eq!(ws.get_cell((6, 3)).unwrap().get_data_type(), "n");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn another_consultants_identical_row_is_not_ours() {
        let mine = SheetRow {
            row: 5,
            date: d(2),
            start: NaiveTime::from_hms_opt(9, 0, 0),
            hours: Some(1.0),
            kind: "Working".into(),
            details: "Toplantı".into(),
            party: "ADBA".into(),
            division: "Trumore".into(),
            consultant: "Kaan Baytur".into(),
        };
        let colleague = SheetRow {
            row: 6,
            consultant: "Ayşe".into(),
            ..mine.clone()
        };
        assert!(!mine.same(&colleague));
        assert!(mine.same(&SheetRow {
            consultant: " kaan baytur ".into(),
            ..colleague.clone()
        }));
        // Danışman sütunu yoksa ya da hücre boşsa içerik yeter.
        assert!(mine.same(&SheetRow {
            consultant: String::new(),
            ..colleague
        }));
    }

    #[test]
    fn cell_times_and_hours_are_read() {
        assert_eq!(cell_time("0.375"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(cell_time("46000.5"), NaiveTime::from_hms_opt(12, 0, 0));
        assert_eq!(cell_time("9:05"), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(cell_time("09:30:00"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(cell_time(""), None);
        assert_eq!(cell_time("öğlen"), None);
        assert_eq!(cell_hours("1,5"), Some(1.5));
        assert_eq!(cell_hours(" 2 "), Some(2.0));
        assert_eq!(cell_hours(""), None);
    }

    #[test]
    fn missing_columns_are_reported() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bos.xlsx");
        umya_spreadsheet::writer::xlsx::write(&umya_spreadsheet::new_file(), &path).unwrap();
        let err = append(&path, "", &[row(1, 9, 1.0, "x")]).unwrap_err();
        assert!(matches!(err, Error::MissingColumn("Date")), "{err}");
        // Hata olunca yedek de alınmaz.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
