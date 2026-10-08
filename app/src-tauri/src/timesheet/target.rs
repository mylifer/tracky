//! Çizelgenin yazıldığı yer (Sheets API, Apps Script ya da Excel dosyası) ve satırların
//! dosya biçimine dönüştürülmesi.

use super::*;

/// Dosyaya yazan işlemler (aktarım, tek satır düzenleme, silme, geri ekleme) sırayla çalışır:
/// tablo yanıtı saniyeler sürebilir; arka arkaya yapılan düzenlemeler reddedilmez, sıraya girer.
pub(super) static FILE_WRITES: tauri::async_runtime::Mutex<()> =
    tauri::async_runtime::Mutex::const_new(());

/// Çizelgenin kayıtlarının yazıldığı yer: Google Sheets (bağlıysa) ya da Excel dosyası.
/// Google bağlıysa ve tablonun bağlantısı biliniyorsa Sheets API ile doğrudan (hızlı); yoksa
/// Apps Script web uygulaması, o da yoksa Excel dosyası.
#[derive(Clone)]
pub(super) enum FileTarget {
    Api { id: String, auth: GoogleAuth },
    Sheets { url: String, token: String },
    Excel { path: std::path::PathBuf },
}

/// Sheets API çağrısı: geçerli erişim anahtarıyla; anahtar reddedilirse bir kez yenilenip
/// yinelenir.
pub(super) fn with_google<T>(
    auth: &GoogleAuth,
    f: impl Fn(&str) -> tracky_xlsx::Result<T>,
) -> tracky_xlsx::Result<T> {
    let token = crate::google::access_token(auth).map_err(tracky_xlsx::Error::Sheets)?;
    match f(&token) {
        Err(tracky_xlsx::Error::Sheets(m)) if m.contains("oturumu geçersiz") => {
            crate::google::forget_access();
            let token = crate::google::access_token(auth).map_err(tracky_xlsx::Error::Sheets)?;
            f(&token)
        }
        r => r,
    }
}

impl FileTarget {
    pub(super) fn of(sheet: &Timesheet, token: &str, google: &GoogleAuth) -> CmdResult<Self> {
        // Tablo bağlantısı yalnızca çizelge Sheets'e bağlıyken (`sheet_url`) geçerlidir: Excel'e
        // geçilmiş çizelgede kalmış eski bağlantı kayıtları eski (belki başka firmanın)
        // tablosuna göndermesin.
        let api = (google.connected() && sheet.sheet_url.is_some())
            .then(|| {
                sheet
                    .sheet_link
                    .as_deref()
                    .and_then(tracky_xlsx::gsheets::spreadsheet_id)
            })
            .flatten();
        match (api, &sheet.sheet_url, &sheet.file_path) {
            (Some(id), _, _) => Ok(Self::Api {
                id,
                auth: google.clone(),
            }),
            (None, Some(url), _) => Ok(Self::Sheets {
                url: url.clone(),
                token: token.to_string(),
            }),
            (None, None, Some(path)) => Ok(Self::Excel { path: path.into() }),
            (None, None, None) => Err(NO_TARGET.into()),
        }
    }

    /// Çizelgenin ve yazıldığı yerin bilgileri (depo kilidi tutulurken).
    pub(super) fn load(store: &Store, timesheet_id: &str) -> CmdResult<(Timesheet, Self)> {
        let config = store.timesheet_config().map_err(err)?;
        let sheet = config.timesheet(timesheet_id).cloned().ok_or(NO_SHEET)?;
        let target = Self::of(&sheet, &config.sheet_token, &crate::google::load(store))?;
        Ok((sheet, target))
    }

    pub(super) async fn list(&self, from: NaiveDate, to: NaiveDate) -> CmdResult<Vec<FileRow>> {
        let target = self.clone();
        let rows = tauri::async_runtime::spawn_blocking(move || match &target {
            Self::Api { id, auth } => {
                with_google(auth, |t| tracky_xlsx::gsheets::list(t, id, from, to))
            }
            Self::Sheets { url, token } => tracky_xlsx::sheets::list(url, token, from, to),
            Self::Excel { path } => tracky_xlsx::list(path, from, to),
        })
        .await
        .map_err(err)?
        .map_err(err)?;
        Ok(rows.into_iter().map(file_row).collect())
    }

    pub(super) async fn update(
        &self,
        consultant: &str,
        expect: &FileRow,
        row: &TimesheetEntry,
    ) -> CmdResult<u32> {
        let (target, consultant) = (self.clone(), consultant.to_string());
        let (expect, row) = (sheet_row(expect), xlsx_row(row));
        tauri::async_runtime::spawn_blocking(move || match &target {
            Self::Api { id, auth } => with_google(auth, |t| {
                tracky_xlsx::gsheets::update(t, id, &consultant, &expect, &row)
            }),
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::update(url, token, &consultant, &expect, &row)
            }
            Self::Excel { path } => tracky_xlsx::update(path, &consultant, &expect, &row),
        })
        .await
        .map_err(err)?
        .map_err(err)
    }

    pub(super) async fn insert(&self, consultant: &str, row: &TimesheetEntry) -> CmdResult<u32> {
        let (target, consultant, row) = (self.clone(), consultant.to_string(), xlsx_row(row));
        tauri::async_runtime::spawn_blocking(move || match &target {
            Self::Api { id, auth } => with_google(auth, |t| {
                tracky_xlsx::gsheets::insert(t, id, &consultant, &row)
            }),
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::insert(url, token, &consultant, &row)
            }
            Self::Excel { path } => tracky_xlsx::insert(path, &consultant, &row),
        })
        .await
        .map_err(err)?
        .map_err(err)
    }

    /// Kum'un aktardığı `entry` dosyada yok mu (elle silinmiş): gününün satırlarından hiçbiri
    /// onunla eşlenmiyor ([`timesheet::link_file_rows`]). Apps Script yolunda hep `false`: betik
    /// aktarılan kaydın kimliğini hatırlar; dosyaya dokunmadan çekilen kayıt yeniden
    /// gönderildiğinde "zaten yazıldı" diye atlanırdı.
    pub(super) async fn lost(&self, entry: &TimesheetEntry) -> CmdResult<bool> {
        if matches!(self, Self::Sheets { .. }) {
            return Ok(false);
        }
        // Danışman süzülmez: danışman adı aktarımdan sonra değiştiyse satır hâlâ dosyadadır;
        // "kayıp" sayılsaydı dosyada kalırken Kum'dan çekilirdi.
        let rows = self.list(entry.date, entry.date).await?;
        Ok(timesheet::link_file_rows(&rows, &[entry])
            .iter()
            .all(Option::is_none))
    }

    /// `id`: satırı Kum aktardıysa kaydın kimliği (Sheets betiği onu unutur; yeniden gönderilebilir).
    pub(super) async fn remove(&self, expect: &FileRow, id: Option<&str>) -> CmdResult<()> {
        let (target, expect, id) = (self.clone(), sheet_row(expect), id.map(str::to_string));
        tauri::async_runtime::spawn_blocking(move || match &target {
            Self::Api { id: sid, auth } => {
                with_google(auth, |t| tracky_xlsx::gsheets::remove(t, sid, &expect))
            }
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::remove(url, token, &expect, id.as_deref())
            }
            Self::Excel { path } => tracky_xlsx::remove(path, &expect),
        })
        .await
        .map_err(err)?
        .map_err(err)
    }
}

/// Dosya satırı çizelgenin danışmanının olabilir: danışmanı yazılı değil ya da aynı (ortak
/// tabloda başka danışmanların satırları ayrılır).
pub(super) fn mine(consultant: &str, r: &FileRow) -> bool {
    let (me, who) = (
        consultant.trim().to_lowercase(),
        r.consultant.trim().to_lowercase(),
    );
    me.is_empty() || who.is_empty() || who == me
}

pub(super) fn file_row(s: tracky_xlsx::SheetRow) -> FileRow {
    FileRow {
        row: s.row,
        date: s.date,
        start: s.start,
        hours: s.hours,
        kind: s.kind,
        details: s.details,
        party: s.party,
        division: s.division,
        consultant: s.consultant,
    }
}

pub(super) fn sheet_row(f: &FileRow) -> tracky_xlsx::SheetRow {
    tracky_xlsx::SheetRow {
        row: f.row,
        date: f.date,
        start: f.start,
        hours: f.hours,
        kind: f.kind.clone(),
        details: f.details.clone(),
        party: f.party.clone(),
        division: f.division.clone(),
        consultant: f.consultant.clone(),
    }
}

/// Kum'un aktardığı kaydın dosyada beklenen satırı: danışmanı satırın yazıldığı ad (bilinmiyorsa
/// çizelgeninki; ayarlarda ad sonradan değişse de satır bulunur); ortak tabloda iş arkadaşının
/// aynı içerikli satırı bulunmasın.
pub(super) fn kum_row(saved: &SavedEntry, row: u32, sheet: &Timesheet) -> FileRow {
    FileRow {
        consultant: saved
            .consultant
            .as_deref()
            .unwrap_or(&sheet.consultant)
            .trim()
            .to_string(),
        ..FileRow::of(&saved.entry, row)
    }
}

/// Kaydın dosyaya yazılan hali.
pub(super) fn xlsx_row(e: &TimesheetEntry) -> tracky_xlsx::Row {
    tracky_xlsx::Row {
        date: e.date,
        start: e.start,
        hours: e.hours,
        kind: e.kind.label().to_string(),
        details: e.details.trim().to_string(),
        party: e.party.trim().to_string(),
        division: e.division.trim().to_string(),
    }
}
