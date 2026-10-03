//! Zaman çizelgesi kayıtlarını kullanıcının kendi Excel dosyasına ekler.
//!
//! Dosyanın düzeni başlık satırındaki sütun adlarından bulunur ("Date", "Day",
//! "Consultant", "Started at", "Amount of Hours", "Type", "Details", "Parties",
//! "… Division"); sütunların yeri değişse de çalışır. Her kayıt için önce o günün
//! önceden doldurulmuş boş satırı (tarih ve danışman yazılı, ayrıntı boş) kullanılır;
//! yetmezse o günün son satırının altına, gün hiç yoksa tarih sırasını bozmayacak
//! yere satır eklenir. Eklenen satırlar biçimini üstteki satırdan alır. Yazmadan önce
//! dosyanın yanına zaman damgalı yedek alınır.

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
struct Columns {
    date: u32,
    day: Option<u32>,
    consultant: Option<u32>,
    start: u32,
    hours: u32,
    kind: u32,
    details: u32,
    party: u32,
    division: u32,
}

const HEADER_ROW: u32 = 1;
const DAY_FORMULA: &str = r#"SWITCH(WEEKDAY(B{r}),1,"Sunday",2,"Monday",3,"Tuesday",4,"Wednesday",5,"Thursday",6,"Friday",7,"Saturday")"#;

fn columns(ws: &Worksheet) -> Result<Columns> {
    let last = ws.get_highest_column().max(1);
    let headers: Vec<(u32, String)> = (1..=last)
        .map(|c| {
            (
                c,
                ws.get_value((c, HEADER_ROW))
                    .split_whitespace()
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

/// Excel tarih seri numarası (1900 sistemi) → tarih.
fn serial_to_date(v: &str) -> Option<NaiveDate> {
    let n: f64 = v.trim().parse().ok()?;
    if !(1.0..2_958_466.0).contains(&n) {
        return None;
    }
    NaiveDate::from_ymd_opt(1899, 12, 30)?.checked_add_days(chrono::Days::new(n.floor() as u64))
}

fn date_to_serial(d: NaiveDate) -> f64 {
    (d - NaiveDate::from_ymd_opt(1899, 12, 30).expect("sabit tarih")).num_days() as f64
}

fn time_to_fraction(t: NaiveTime) -> f64 {
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
    let ranked = |col: u32| -> Vec<String> {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in (HEADER_ROW + 1)..=ws.get_highest_row() {
            let v = ws.get_value((col, r)).trim().to_string();
            if !v.is_empty() {
                *counts.entry(v).or_default() += 1;
            }
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(k, _)| k).collect()
    };
    let header = ws.get_value((cols.division, HEADER_ROW));
    let company = header
        .split_whitespace()
        .take_while(|w| !w.eq_ignore_ascii_case("division"))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(Template {
        company: (!company.is_empty()).then_some(company),
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

fn write_rows(book: &mut Spreadsheet, consultant: &str, rows: &[Row]) -> Result<(usize, usize)> {
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    let cols = columns(ws)?;
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by_key(|r| (r.date, r.start));
    let (mut filled, mut inserted) = (0, 0);
    for row in sorted {
        let last = ws.get_highest_row();
        let date_at = |ws: &Worksheet, r: u32| serial_to_date(&ws.get_value((cols.date, r)));
        let empty = |ws: &Worksheet, r: u32| {
            ws.get_value((cols.details, r)).trim().is_empty()
                && ws.get_value((cols.kind, r)).trim().is_empty()
        };
        let target = match ((HEADER_ROW + 1)..=last)
            .find(|&r| date_at(ws, r) == Some(row.date) && empty(ws, r))
        {
            Some(r) => {
                filled += 1;
                r
            }
            None => {
                // O günün son satırının, yoksa daha önceki son tarihin altına.
                let after = ((HEADER_ROW + 1)..=last)
                    .filter(|&r| date_at(ws, r).is_some_and(|d| d <= row.date))
                    .max()
                    .unwrap_or(HEADER_ROW);
                let r = after + 1;
                ws.insert_new_row(&r, &1);
                copy_row_style(ws, after.max(HEADER_ROW + 1).min(r - 1), r, cols);
                inserted += 1;
                r
            }
        };
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
        ws.get_cell_mut((c, r)).set_value(consultant.trim());
    }
    ws.get_cell_mut((cols.start, r))
        .set_value_number(time_to_fraction(row.start));
    ws.get_style_mut((cols.start, r))
        .get_number_format_mut()
        .set_format_code("hh:mm");
    // Tam değer yazılır (toplamlar kesin olsun); yalnızca gösterim iki ondalık.
    ws.get_cell_mut((cols.hours, r)).set_value_number(row.hours);
    ws.get_style_mut((cols.hours, r))
        .get_number_format_mut()
        .set_format_code("0.00");
    ws.get_cell_mut((cols.kind, r)).set_value(row.kind.as_str());
    ws.get_cell_mut((cols.details, r))
        .set_value(row.details.as_str());
    ws.get_cell_mut((cols.party, r))
        .set_value(row.party.as_str());
    ws.get_cell_mut((cols.division, r))
        .set_value(row.division.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

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
