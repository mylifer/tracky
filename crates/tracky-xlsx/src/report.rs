//! Aylık müşteri raporunu yeni bir Excel dosyasına yazar: satırlar projeler, sütunlar ayın
//! günleri, sağda ve altta toplamlar. Hafta sonu sütunları gri, boş günler boş bırakılır.

use std::path::Path;

use chrono::{Datelike, NaiveDate, Weekday};
use umya_spreadsheet::helper::coordinate::string_from_column_index;
use umya_spreadsheet::{HorizontalAlignmentValues, Worksheet};

use crate::{Error, Result};

/// Raporun bir satırı.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixRow {
    /// Müşteri adı; müşterisiz projede boş.
    pub client: String,
    pub project: String,
    /// Gün başına saat, [`Matrix::days`] sırasıyla.
    pub hours: Vec<f64>,
}

/// Yazılacak rapor.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    /// İlk satır (örn. "Togg · Eylül 2026").
    pub title: String,
    /// İkinci satır (örn. "Kaynak: zaman çizelgesi").
    pub subtitle: String,
    pub days: Vec<NaiveDate>,
    pub rows: Vec<MatrixRow>,
}

/// Gün başlıklarının bulunduğu satır; altında haftanın günü, sonra projeler.
const DAY_ROW: u32 = 4;
const FIRST_ROW: u32 = DAY_ROW + 2;
/// İlk gün sütunu (A: müşteri, B: proje).
const FIRST_DAY_COL: u32 = 3;
const HOURS_FORMAT: &str = "0.00";
const WEEKEND_FILL: &str = "FFF1F1F1";
const TOTAL_FILL: &str = "FFE8E2D6";

fn weekday_short(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "Pt",
        Weekday::Tue => "Sa",
        Weekday::Wed => "Ça",
        Weekday::Thu => "Pe",
        Weekday::Fri => "Cu",
        Weekday::Sat => "Ct",
        Weekday::Sun => "Pz",
    }
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// `matrix`'i `path`'e yazar (dosya varsa üzerine).
pub fn write_matrix(path: &Path, matrix: &Matrix) -> Result<()> {
    let mut book = umya_spreadsheet::new_file();
    let ws = book.get_sheet_mut(&0).ok_or(Error::NoSheet)?;
    ws.set_name("Rapor");
    fill(ws, matrix);
    umya_spreadsheet::writer::xlsx::write(&book, path).map_err(|e| Error::Write(e.to_string()))
}

fn fill(ws: &mut Worksheet, m: &Matrix) {
    let n = m.days.len() as u32;
    let total_col = FIRST_DAY_COL + n;
    ws.get_cell_mut((1, 1)).set_value(m.title.as_str());
    let title = ws.get_style_mut((1, 1)).get_font_mut();
    title.set_bold(true);
    title.set_size(14.0);
    ws.get_cell_mut((1, 2)).set_value(m.subtitle.as_str());

    let header = |ws: &mut Worksheet, col: u32, row: u32, text: &str| {
        ws.get_cell_mut((col, row)).set_value(text);
        let style = ws.get_style_mut((col, row));
        style.get_font_mut().set_bold(true);
        style
            .get_alignment_mut()
            .set_horizontal(HorizontalAlignmentValues::Center);
    };
    header(ws, 1, DAY_ROW, "Müşteri");
    header(ws, 2, DAY_ROW, "Proje");
    for (i, d) in m.days.iter().enumerate() {
        let col = FIRST_DAY_COL + i as u32;
        header(ws, col, DAY_ROW, &d.day().to_string());
        header(ws, col, DAY_ROW + 1, weekday_short(*d));
    }
    header(ws, total_col, DAY_ROW, "Toplam");

    let number = |ws: &mut Worksheet, col: u32, row: u32, v: f64, bold: bool| {
        ws.get_cell_mut((col, row)).set_value_number(v);
        let style = ws.get_style_mut((col, row));
        style.get_number_format_mut().set_format_code(HOURS_FORMAT);
        if bold {
            style.get_font_mut().set_bold(true);
        }
    };
    let mut day_totals = vec![0.0; m.days.len()];
    for (r, row) in m.rows.iter().enumerate() {
        let line = FIRST_ROW + r as u32;
        ws.get_cell_mut((1, line)).set_value(row.client.as_str());
        ws.get_cell_mut((2, line)).set_value(row.project.as_str());
        let mut total = 0.0;
        for (i, h) in row.hours.iter().enumerate().take(m.days.len()) {
            if *h > 0.0 {
                number(ws, FIRST_DAY_COL + i as u32, line, *h, false);
                total += h;
                day_totals[i] += h;
            }
        }
        number(ws, total_col, line, total, true);
    }

    let last = FIRST_ROW + m.rows.len() as u32;
    ws.get_cell_mut((2, last)).set_value("Toplam");
    ws.get_style_mut((2, last)).get_font_mut().set_bold(true);
    for (i, v) in day_totals.iter().enumerate() {
        if *v > 0.0 {
            number(ws, FIRST_DAY_COL + i as u32, last, *v, true);
        }
    }
    number(ws, total_col, last, day_totals.iter().sum(), true);
    for col in 1..=total_col {
        ws.get_style_mut((col, last))
            .set_background_color(TOTAL_FILL);
    }

    // Hafta sonu sütunları (başlıktan toplam satırının üstüne kadar).
    for (i, d) in m.days.iter().enumerate() {
        if is_weekend(*d) {
            for row in DAY_ROW..last {
                ws.get_style_mut((FIRST_DAY_COL + i as u32, row))
                    .set_background_color(WEEKEND_FILL);
            }
        }
    }

    ws.get_column_dimension_mut("A").set_width(18.0);
    ws.get_column_dimension_mut("B").set_width(28.0);
    for col in FIRST_DAY_COL..total_col {
        ws.get_column_dimension_mut(&string_from_column_index(&col))
            .set_width(6.0);
    }
    ws.get_column_dimension_mut(&string_from_column_index(&total_col))
        .set_width(9.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_matrix_that_reads_back() {
        let dir = std::env::temp_dir().join(format!("kum-xlsx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rapor.xlsx");
        let days: Vec<NaiveDate> = (1..=30)
            .map(|d| NaiveDate::from_ymd_opt(2026, 9, d).unwrap())
            .collect();
        let mut kum = vec![0.0; 30];
        kum[0] = 2.5;
        kum[4] = 1.25;
        let mut loy = vec![0.0; 30];
        loy[0] = 0.75;
        let matrix = Matrix {
            title: "Togg · Eylül 2026".into(),
            subtitle: "Kaynak: zaman çizelgesi".into(),
            days,
            rows: vec![
                MatrixRow {
                    client: "Togg".into(),
                    project: "Kum".into(),
                    hours: kum,
                },
                MatrixRow {
                    client: String::new(),
                    project: "Loyalty".into(),
                    hours: loy,
                },
            ],
        };
        write_matrix(&path, &matrix).unwrap();

        let book = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
        let ws = book.get_sheet(&0).unwrap();
        let num = |c: u32, r: u32| ws.get_value((c, r)).parse::<f64>().ok();
        assert_eq!(ws.get_name(), "Rapor");
        assert_eq!(ws.get_value((1, 1)), "Togg · Eylül 2026");
        // Başlık: gün numarası ve haftanın günü (1 Eylül 2026 salı); toplam sütunu 33.
        assert_eq!(ws.get_value((3, 4)), "1");
        assert_eq!(ws.get_value((3, 5)), "Sa");
        assert_eq!(ws.get_value((33, 4)), "Toplam");
        assert_eq!(ws.get_value((2, 6)), "Kum");
        assert_eq!(
            (num(3, 6), num(7, 6), num(33, 6)),
            (Some(2.5), Some(1.25), Some(3.75))
        );
        // Boş gün boş kalır.
        assert_eq!(ws.get_value((4, 6)), "");
        assert_eq!(ws.get_value((1, 7)), "");
        // Toplam satırı.
        assert_eq!(ws.get_value((2, 8)), "Toplam");
        assert_eq!((num(3, 8), num(33, 8)), (Some(3.25), Some(4.5)));
        assert_eq!(
            ws.get_style((3, 6))
                .get_number_format()
                .unwrap()
                .get_format_code(),
            "0.00"
        );
        // 5 Eylül cumartesi: gri.
        assert!(ws.get_style((7, 4)).get_background_color().is_some());
        assert!(ws.get_style((3, 4)).get_background_color().is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
