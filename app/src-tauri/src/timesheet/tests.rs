use super::*;

const LINK: &str =
    "https://docs.google.com/spreadsheets/d/1KoL_-q6Cxq1yqfqCG61rSZHsxHpnsgyQtz_QGIZrR9U/edit";

#[test]
fn a_stale_sheet_link_does_not_send_to_google() {
    let google = GoogleAuth {
        client_id: "x.apps.googleusercontent.com".into(),
        refresh_token: Some("r".into()),
        ..Default::default()
    };
    // Excel'e geçilmiş çizelgede kalmış eski tablo bağlantısı: Excel dosyasına yazılır.
    let excel = Timesheet {
        file_path: Some("/tmp/togg.xlsx".into()),
        sheet_link: Some(LINK.into()),
        ..Default::default()
    };
    assert!(matches!(
        FileTarget::of(&excel, "t", &google),
        Ok(FileTarget::Excel { .. })
    ));
    let none = Timesheet {
        sheet_link: Some(LINK.into()),
        ..Default::default()
    };
    assert!(FileTarget::of(&none, "t", &google).is_err());
    // Sheets'e bağlıyken Google bağlıysa API, değilse betik.
    let sheets = Timesheet {
        sheet_url: Some("https://script.google.com/macros/s/x/exec".into()),
        ..excel
    };
    assert!(matches!(
        FileTarget::of(&sheets, "t", &google),
        Ok(FileTarget::Api { .. })
    ));
    assert!(matches!(
        FileTarget::of(&sheets, "t", &GoogleAuth::default()),
        Ok(FileTarget::Sheets { .. })
    ));
}

#[test]
fn rows_of_other_consultants_are_not_mine() {
    let row = |who: &str| FileRow {
        consultant: who.into(),
        ..Default::default()
    };
    assert!(mine("Kaan Baytur", &row(" kaan baytur ")));
    assert!(mine("Kaan Baytur", &row("")));
    assert!(mine("", &row("Ayşe")));
    assert!(!mine("Kaan Baytur", &row("Ayşe")));
}
