//! Çizelge kurulumu: Excel dosyası seçme, şablonu içe aktarma, Google Sheets bağlantısı ve
//! çizelgeyi kaldırma.

use super::*;

/// Excel dosyası seçtirir; vazgeçilirse `None`.
#[tauri::command]
pub async fn pick_timesheet_file(app: AppHandle) -> CmdResult<Option<String>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Zaman çizelgesi Excel dosyası")
            .add_filter("Excel", &["xlsx"])
            .blocking_pick_file()
    })
    .await
    .map_err(err)?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    config: TimesheetConfig,
    /// İçe aktarılan (yeni ya da güncellenen) çizelge.
    timesheet_id: String,
    /// Yeni oluşturulan proje adları.
    created: Vec<String>,
    details: usize,
}

/// İçe aktarılan dosyanın çizelgesi: `id` verilmişse o, yoksa aynı dosyaya ya da tabloya bağlı
/// çizelge (`same`; aynı dosyayı yeniden seçmek ikinci çizelge açmasın), o da yoksa sona eklenen
/// yeni çizelge. (sıra, yeni mi)
pub(super) fn sheet_slot(
    config: &mut TimesheetConfig,
    id: Option<&str>,
    same: impl Fn(&Timesheet) -> bool,
) -> (usize, bool) {
    let found = match id {
        Some(id) => config.timesheets.iter().position(|t| t.id == id),
        None => config.timesheets.iter().position(same),
    };
    match found {
        Some(i) => (i, false),
        None => {
            config.timesheets.push(Timesheet {
                id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            });
            (config.timesheets.len() - 1, true)
        }
    }
}

/// Şablonu içe aktarır ve dosyayı çizelgenin (`timesheet_id` yoksa yeni çizelgenin) Excel
/// dosyası yapar: firma, danışman, taraf ve birimler dosyadan alınır; yeni çizelgede birimler
/// proje olur ([`apply_template`]).
#[tauri::command]
pub async fn import_timesheet_template(
    app: AppHandle,
    timesheet_id: Option<String>,
    path: String,
) -> CmdResult<Imported> {
    let template = {
        let path = std::path::PathBuf::from(&path);
        tauri::async_runtime::spawn_blocking(move || tracky_xlsx::inspect(&path))
            .await
            .map_err(err)?
            .map_err(err)?
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    let (i, new) = sheet_slot(&mut config, timesheet_id.as_deref(), |t| {
        t.file_path.as_deref() == Some(path.as_str())
    });
    config.timesheets[i].file_path = Some(path);
    // Excel seçildi: kayıtlar bundan sonra bu dosyaya gider.
    config.timesheets[i].sheet_url = None;
    config.timesheets[i].sheet_link = None;
    apply_template(&store, config, i, template, new)
}

/// Şablon bilgilerini `i` çizelgesine işler: firma, danışman, taraf ve birimler. Yeni çizelgede
/// her birim (yoksa) proje olur ve başka çizelgeye bağlı değilse bu çizelgeye bağlanır; firma
/// müşteri olur ve müşterisi olmayan bu projeler ona bağlanır. Var olan çizelgeye yeniden içe
/// aktarmada proje oluşturulmaz (kullanıcının sildiği birim projeleri geri gelmesin).
pub(super) fn apply_template(
    store: &Store,
    mut config: TimesheetConfig,
    i: usize,
    template: tracky_xlsx::Template,
    new: bool,
) -> CmdResult<Imported> {
    let sheet = &mut config.timesheets[i];
    if let Some(c) = template.company {
        sheet.company = c;
    }
    if let Some(c) = template.consultant {
        sheet.consultant = c;
    }
    if let Some(p) = template.parties.first() {
        sheet.default_party = p.clone();
    }
    sheet.divisions.extend(template.divisions.iter().cloned());
    let company = sheet.company.trim().to_string();
    let mut created = Vec::new();
    if new {
        // Firma müşteri olur; dosyadaki birimlerin (projelerin) müşterisi yoksa ona bağlanır.
        let client_id = match company.as_str() {
            "" => None,
            company => {
                let clients = store.clients().map_err(err)?;
                let id = match clients
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(company))
                {
                    Some(c) => c.id.clone(),
                    None => {
                        let c = tracky_core::Client {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: company.to_string(),
                        };
                        store.upsert_client(&c, clients.len() as i64).map_err(err)?;
                        c.id
                    }
                };
                Some(id)
            }
        };
        let linked = store.project_clients().map_err(err)?;
        for division in &template.divisions {
            let tags = store.tags().map_err(err)?;
            let existing = tags
                .iter()
                .find(|t| t.kind == TagKind::Project && t.name.eq_ignore_ascii_case(division));
            let id = match existing {
                Some(t) => t.id.clone(),
                None => {
                    let tag = Tag {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: TagKind::Project,
                        name: division.clone(),
                        color: (tags.len() % 8) as u8 + 1,
                    };
                    store.upsert_tag(&tag, tags.len() as i64).map_err(err)?;
                    // Kuralsız proje süre toplamaz; adı başlıkta aranan sözcük olur.
                    store
                        .upsert_rule(&Rule {
                            id: uuid::Uuid::new_v4().to_string(),
                            tag_id: tag.id.clone(),
                            field: RuleField::Title,
                            pattern: division.clone(),
                        })
                        .map_err(err)?;
                    created.push(division.clone());
                    tag.id
                }
            };
            if let Some(client) = &client_id
                && !linked.contains_key(&id)
            {
                store.set_project_client(&id, Some(client)).map_err(err)?;
            }
            if config.timesheet_of(&id).is_none() {
                config.timesheets[i].projects.push(ProjectMapping {
                    project_id: id,
                    division: division.clone(),
                    party: None,
                    default_details: None,
                });
            }
        }
    }
    let timesheet_id = config.timesheets[i].id.clone();
    store.save_timesheet_config(&config).map_err(err)?;
    // Açıklama önerileri: dosyanınkiler önce, önceki çizelgelerinkiler arkada.
    let mut details: Vec<String> = template.details;
    details.extend(
        store
            .setting::<Vec<String>>(DETAILS_KEY)
            .map_err(err)?
            .unwrap_or_default(),
    );
    let mut seen = std::collections::HashSet::new();
    details.retain(|d| seen.insert(d.to_lowercase()));
    details.truncate(MAX_DETAILS);
    store.save_setting(DETAILS_KEY, &details).map_err(err)?;
    Ok(Imported {
        config: store.timesheet_config().map_err(err)?,
        timesheet_id,
        created,
        details: details.len(),
    })
}

/// Apps Script anahtarını (yoksa üretip) döndürür; bütün tablolarda aynıdır.
pub(super) fn sheet_token(store: &Store) -> CmdResult<String> {
    let mut config = store.timesheet_config().map_err(err)?;
    if config.sheet_token.is_empty() {
        config.sheet_token = uuid::Uuid::new_v4().simple().to_string();
        store.save_timesheet_config(&config).map_err(err)?;
    }
    Ok(config.sheet_token)
}

/// Google Sheets tablosuna eklenecek Apps Script (bu kuruluma özgü anahtarla).
#[tauri::command]
pub async fn sheet_script(app: AppHandle) -> CmdResult<String> {
    let token = sheet_token(&lock(&app.state::<Shared>().store))?;
    Ok(tracky_xlsx::sheets::script(&token))
}

/// Çizelgeyi (`timesheet_id` yoksa yeni çizelgeyi) Google Sheets'e bağlar: web uygulamasını
/// dener, tablodan şablon bilgilerini içe aktarır; çizelgenin kayıtları bundan sonra bu tabloya
/// gider.
#[tauri::command]
pub async fn connect_sheet(
    app: AppHandle,
    timesheet_id: Option<String>,
    url: String,
    link: Option<String>,
) -> CmdResult<Imported> {
    let url = tracky_xlsx::sheets::check_url(&url).map_err(err)?;
    let token = sheet_token(&lock(&app.state::<Shared>().store))?;
    let template = {
        let url = url.clone();
        tauri::async_runtime::spawn_blocking(move || tracky_xlsx::sheets::inspect(&url, &token))
            .await
            .map_err(err)?
            .map_err(err)?
    };
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    let (i, new) = sheet_slot(&mut config, timesheet_id.as_deref(), |t| {
        t.sheet_url.as_deref() == Some(url.as_str())
    });
    config.timesheets[i].sheet_url = Some(url);
    config.timesheets[i].sheet_link = link.map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    apply_template(&store, config, i, template, new)
}

/// Çizelgenin Google Sheets bağlantısını kaldırır; kayıtları yeniden Excel dosyasına (seçiliyse)
/// gider.
#[tauri::command]
pub async fn disconnect_sheet(app: AppHandle, timesheet_id: String) -> CmdResult<TimesheetConfig> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut config = store.timesheet_config().map_err(err)?;
    let sheet = config
        .timesheets
        .iter_mut()
        .find(|t| t.id == timesheet_id)
        .ok_or(NO_SHEET)?;
    sheet.sheet_url = None;
    sheet.sheet_link = None;
    store.save_timesheet_config(&config).map_err(err)?;
    Ok(config)
}

/// Çizelgenin tablosunu tarayıcıda (Google Sheets) ya da varsayılan uygulamada (Excel) açar.
#[tauri::command]
pub async fn open_timesheet(app: AppHandle, timesheet_id: String) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    let config = lock(&shared.store).timesheet_config().map_err(err)?;
    let sheet = config.timesheet(&timesheet_id).ok_or(NO_SHEET)?;
    let target = if sheet.sheet_url.is_some() {
        sheet
            .sheet_link
            .as_deref()
            .filter(|l| l.starts_with("https://docs.google.com/"))
            .ok_or("Tablonun bağlantısı (docs.google.com/…) girilmemiş; Ayarlar → Zaman çizelgeleri.")?
    } else {
        sheet.file_path.as_deref().ok_or("Kayıtların yazılacağı dosya seçilmedi.")?
    };
    crate::google::open_browser(target);
    Ok(())
}

/// Çizelgeyi kaldırır; projeleri hiçbir çizelgeye gitmez. Kaydedilmiş satırlar silinmez.
#[tauri::command]
pub async fn remove_timesheet(app: AppHandle, timesheet_id: String) -> CmdResult<TimesheetConfig> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    not_exporting()?;
    let mut config = store.timesheet_config().map_err(err)?;
    config.timesheets.retain(|t| t.id != timesheet_id);
    store.save_timesheet_config(&config).map_err(err)?;
    Ok(config)
}
