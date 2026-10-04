//! Kullanıcının gerçek şablonuyla (kişisel veri, depoda yok) uçtan uca deneme. Dosya
//! yoksa atlanır: `KUM_SABLON=/yol/sablon.xlsx cargo test -p tracky-xlsx`; varsayılan
//! olarak depo kökündeki `sablon.xlsx` aranır.

use std::path::PathBuf;

use chrono::{NaiveDate, NaiveTime};
use tracky_xlsx::{Row, append, inspect};
use umya_spreadsheet::Worksheet;

fn template() -> Option<PathBuf> {
    let path = std::env::var_os("KUM_SABLON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sablon.xlsx"));
    path.exists().then_some(path)
}

fn date(ws: &Worksheet, r: u32) -> Option<NaiveDate> {
    let n: f64 = ws.get_value((2, r)).trim().parse().ok()?;
    NaiveDate::from_ymd_opt(1899, 12, 30)?.checked_add_days(chrono::Days::new(n as u64))
}

/// Satırın karşılaştırılan içeriği: tarih, danışman, başlangıç, saat, tür, ayrıntı, taraf, birim.
fn line(ws: &Worksheet, r: u32) -> Vec<String> {
    [2u32, 4, 5, 6, 7, 8, 9, 10]
        .iter()
        .map(|&c| ws.get_value((c, r)))
        .collect()
}

/// Ayrıntı sütunundaki bağlantılar: (hücre metni, adres).
fn links(ws: &Worksheet) -> Vec<(String, String)> {
    (1..=ws.get_highest_row())
        .filter_map(|r| {
            let link = ws.get_cell((8, r))?.get_hyperlink()?;
            Some((ws.get_value((8, r)), link.get_url().to_string()))
        })
        .collect()
}

/// Gün sütununun formülleri, satır numarasıyla (2'den başlar).
fn days(ws: &Worksheet) -> Vec<(u32, String)> {
    (2..=ws.get_highest_row())
        .map(|r| {
            let f = ws
                .get_cell((3, r))
                .map(|c| c.get_formula().to_string())
                .unwrap_or_default();
            (r, f)
        })
        .collect()
}

#[test]
fn appending_to_real_template_keeps_everything_else() {
    let Some(src) = template() else {
        eprintln!("sablon.xlsx yok, atlandı");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kum-real-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sablon.xlsx");
    std::fs::copy(&src, &path).unwrap();

    let t = inspect(&path).unwrap();
    let before = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
    let ws = before.get_sheet(&0).unwrap();
    let last = ws.get_highest_row();
    // Ortalarda, kaydı dolu bir gün: ekleme araya satır sokar (paylaşılan formül aralığının içine).
    let at = (40..last)
        .find(|&r| date(ws, r).is_some() && !ws.get_value((8, r)).trim().is_empty())
        .unwrap();
    let day = date(ws, at).unwrap();
    let old: Vec<Vec<String>> = (2..=last).map(|r| line(ws, r)).collect();
    let old_links = links(ws);
    let old_days = days(ws);

    let row = Row {
        date: day,
        start: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
        hours: 0.5,
        kind: "Working".into(),
        details: "KUM-TEST".into(),
        party: t.parties.first().cloned().unwrap_or_default(),
        division: t.divisions.first().cloned().unwrap_or_default(),
    };
    let done = append(&path, t.consultant.as_deref().unwrap_or(""), &[row]).unwrap();
    assert_eq!(done.filled + done.inserted, 1);

    let after = umya_spreadsheet::reader::xlsx::read(&path).unwrap();
    let ws = after.get_sheet(&0).unwrap();
    let new_at = (2..=ws.get_highest_row())
        .find(|&r| ws.get_value((8, r)) == "KUM-TEST")
        .unwrap();
    assert_eq!(date(ws, new_at), Some(day));
    // Eklenen satır dışında her satır aynı içerikle, aynı sırada duruyor.
    let rest: Vec<Vec<String>> = (2..=ws.get_highest_row())
        .filter(|&r| r != new_at)
        .map(|r| line(ws, r))
        .take(old.len())
        .collect();
    for (i, (a, b)) in old.iter().zip(&rest).enumerate() {
        assert_eq!(a, b, "satır {} değişti", i + 2);
    }
    // Gün sütunu: formüllü satırlar formüllü kalıyor, formül kendi satırının tarihine başvuruyor
    // (paylaşılan formül aralığına satır eklense de); eklenen satır da formül alıyor.
    let now_days = days(ws);
    let shifted = |r: u32| if r >= new_at { r + 1 } else { r };
    for (r, _) in old_days.iter().filter(|(_, f)| !f.is_empty()) {
        let r2 = shifted(*r);
        let f = &now_days[(r2 - 2) as usize].1;
        assert!(
            f.contains(&format!("B{r2}(")) || f.contains(&format!("B{r2})")),
            "C{r2} (eski C{r}) formülü: {f:?}"
        );
    }
    assert!(
        now_days[(new_at - 2) as usize]
            .1
            .contains(&format!("B{new_at})"))
    );
    // Bağlantılar kaymadan aynı metne bağlı.
    let now = links(ws);
    let (mut a, mut b) = (old_links, now);
    a.sort();
    b.sort();
    assert_eq!(a, b, "bağlantılar kaydı");
    // Excel'de elle açıp bakmak için: KUM_SABLON_OUT=/yol/cikti.xlsx
    if let Some(out) = std::env::var_os("KUM_SABLON_OUT") {
        std::fs::copy(&path, out).unwrap();
    }
    std::fs::remove_dir_all(dir).ok();
}
