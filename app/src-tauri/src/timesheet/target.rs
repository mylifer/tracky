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
    /// `script`: tablonun Apps Script bağlantısı (adres, anahtar); bağlı Google hesabının
    /// tabloda düzenleme yetkisi yoksa ona geçilir.
    Api {
        id: String,
        auth: GoogleAuth,
        script: (String, String),
    },
    Sheets {
        url: String,
        token: String,
    },
    Excel {
        path: std::path::PathBuf,
    },
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

/// Bağlı Google hesabının düzenleme yetkisi olmadığı tablolar: bu oturumda betikle yazılır.
pub(super) static DENIED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Sheets API, hesabın tabloda düzenleme yetkisi olmadığı için mi reddetti.
pub(super) fn denied(e: &tracky_xlsx::Error) -> bool {
    matches!(e, tracky_xlsx::Error::Sheets(m) if m.contains("düzenleme yetkisi yok"))
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
        // Hesabın yetkisi olmadığı anlaşılan tablo bu oturumda doğrudan betikle yazılır.
        let api = api.filter(|id| !crate::lock(&DENIED).contains(id));
        match (api, &sheet.sheet_url, &sheet.file_path) {
            (Some(id), Some(url), _) => Ok(Self::Api {
                id,
                auth: google.clone(),
                script: (url.clone(), token.to_string()),
            }),
            (_, Some(url), _) => Ok(Self::Sheets {
                url: url.clone(),
                token: token.to_string(),
            }),
            (_, None, Some(path)) => Ok(Self::Excel { path: path.into() }),
            (_, None, None) => Err(NO_TARGET.into()),
        }
    }

    /// Çizelgenin ve yazıldığı yerin bilgileri (depo kilidi tutulurken).
    pub(super) fn load(store: &Store, timesheet_id: &str) -> CmdResult<(Timesheet, Self)> {
        let config = store.timesheet_config().map_err(err)?;
        let sheet = config.timesheet(timesheet_id).cloned().ok_or(NO_SHEET)?;
        let target = Self::of(&sheet, &config.sheet_token, &crate::google::load(store))?;
        Ok((sheet, target))
    }

    /// `f`'yi hedefte çalıştırır. Bağlı Google hesabının tabloda düzenleme yetkisi yoksa
    /// (tablo başka hesapla paylaşılmış) aynı işi Apps Script ile yapar; dönen hedef işin
    /// yapıldığı yerdir.
    pub(super) async fn run<T: Send + 'static>(
        &self,
        f: impl Fn(&Self) -> tracky_xlsx::Result<T> + Send + 'static,
    ) -> CmdResult<(T, Self)> {
        let target = self.clone();
        tauri::async_runtime::spawn_blocking(move || match (f(&target), &target) {
            (Err(e), Self::Api { id, script, .. }) if denied(&e) => {
                log_info!("Google hesabının tabloya yetkisi yok, Apps Script ile yazılıyor");
                crate::lock(&DENIED).push(id.clone());
                let (url, token) = script.clone();
                let by_script = Self::Sheets { url, token };
                f(&by_script).map(|r| (r, by_script))
            }
            (r, _) => r.map(|r| (r, target)),
        })
        .await
        .map_err(err)?
        .map_err(err)
    }

    pub(super) async fn list(&self, from: NaiveDate, to: NaiveDate) -> CmdResult<Vec<FileRow>> {
        self.rows(from, to).await.map(|(rows, _)| rows)
    }

    /// Dosyadaki satırlar ve okundukları hedef.
    async fn rows(&self, from: NaiveDate, to: NaiveDate) -> CmdResult<(Vec<FileRow>, Self)> {
        let (rows, used) = self
            .run(move |target| match target {
                Self::Api { id, auth, .. } => {
                    with_google(auth, |t| tracky_xlsx::gsheets::list(t, id, from, to))
                }
                Self::Sheets { url, token } => tracky_xlsx::sheets::list(url, token, from, to),
                Self::Excel { path } => tracky_xlsx::list(path, from, to),
            })
            .await?;
        Ok((rows.into_iter().map(file_row).collect(), used))
    }

    pub(super) async fn update(
        &self,
        consultant: &str,
        expect: &FileRow,
        row: &TimesheetEntry,
    ) -> CmdResult<u32> {
        let consultant = consultant.to_string();
        let (expect, row) = (sheet_row(expect), xlsx_row(row));
        self.run(move |target| match target {
            Self::Api { id, auth, .. } => with_google(auth, |t| {
                tracky_xlsx::gsheets::update(t, id, &consultant, &expect, &row)
            }),
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::update(url, token, &consultant, &expect, &row)
            }
            Self::Excel { path } => tracky_xlsx::update(path, &consultant, &expect, &row),
        })
        .await
        .map(|(r, _)| r)
    }

    pub(super) async fn insert(&self, consultant: &str, row: &TimesheetEntry) -> CmdResult<u32> {
        let (consultant, row) = (consultant.to_string(), xlsx_row(row));
        self.run(move |target| match target {
            Self::Api { id, auth, .. } => with_google(auth, |t| {
                tracky_xlsx::gsheets::insert(t, id, &consultant, &row)
            }),
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::insert(url, token, &consultant, &row)
            }
            Self::Excel { path } => tracky_xlsx::insert(path, &consultant, &row),
        })
        .await
        .map(|(r, _)| r)
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
        let (rows, used) = self.rows(entry.date, entry.date).await?;
        // Yetkisizlikten betiğe geçildiyse betik yolundaki gibi: kayıp sayılmaz.
        if matches!(used, Self::Sheets { .. }) {
            return Ok(false);
        }
        Ok(timesheet::link_file_rows(&rows, &[entry])
            .iter()
            .all(Option::is_none))
    }

    /// `id`: satırı Kum aktardıysa kaydın kimliği (Sheets betiği onu unutur; yeniden gönderilebilir).
    pub(super) async fn remove(&self, expect: &FileRow, id: Option<&str>) -> CmdResult<()> {
        let (expect, id) = (sheet_row(expect), id.map(str::to_string));
        self.run(move |target| match target {
            Self::Api { id: sid, auth, .. } => {
                with_google(auth, |t| tracky_xlsx::gsheets::remove(t, sid, &expect))
            }
            Self::Sheets { url, token } => {
                tracky_xlsx::sheets::remove(url, token, &expect, id.as_deref())
            }
            Self::Excel { path } => tracky_xlsx::remove(path, &expect),
        })
        .await
        .map(|(r, _)| r)
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
