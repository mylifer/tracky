//! Geri alınabilir düzenlemeler ve atanmamış süreyi gözden geçirme komutları.
//!
//! Her düzenleme öncesinin durumunu bir geri alma kaydına yazar ve arayüze kaydın numarasını
//! döndürür; arayüz "Geri al" bildiriminde bu numarayla [`undo`] çağırır. Kayıtlar yalnızca
//! bellekte tutulur (uygulama kapanınca silinir), en yenileri saklanır.

use std::sync::Mutex;

use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tracky_core::inbox::{RulePreview, Unassigned};
use tracky_core::store::EditSnapshot;
use tracky_core::{Rule, RuleField, Store};

use crate::lock;
use crate::tracking::{Shared, local_midnight};

type CmdResult<T> = Result<T, String>;

/// Saklanan en çok geri alma kaydı.
const MAX_UNDO: usize = 30;
/// Kural önizlemesi bu kadar günlük geçmişe bakar.
const PREVIEW_DAYS: i64 = 30;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Bir düzenlemeyi geri almak için gerekenler.
pub enum UndoOp {
    Sessions(EditSnapshot),
    AddedRule(String),
    DeletedRule(Rule),
    DeletedTag { id: String, at: DateTime<Utc> },
}

/// Geri alma kayıtları: son verilen numara ve (numara, işlemler) listesi, eskiden yeniye.
#[derive(Default)]
struct Entries {
    last: u64,
    list: Vec<(u64, Vec<UndoOp>)>,
}

#[derive(Default)]
pub struct UndoLog(Mutex<Entries>);

impl UndoLog {
    /// Kaydı ekler, numarasını döndürür.
    pub fn push(&self, ops: Vec<UndoOp>) -> u64 {
        let mut log = lock(&self.0);
        log.last += 1;
        let id = log.last;
        log.list.push((id, ops));
        if log.list.len() > MAX_UNDO {
            log.list.remove(0);
        }
        id
    }

    fn take(&self, id: u64) -> Option<Vec<UndoOp>> {
        let mut log = lock(&self.0);
        let i = log.list.iter().position(|(n, _)| *n == id)?;
        Some(log.list.remove(i).1)
    }
}

/// Değişiklik sayısı ve geri alma numarası.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edited {
    pub changed: usize,
    pub undo: u64,
}

pub fn record(app: &AppHandle, ops: Vec<UndoOp>) -> u64 {
    app.state::<UndoLog>().push(ops)
}

fn apply(store: &Store, op: &UndoOp) -> tracky_core::store::Result<()> {
    match op {
        UndoOp::Sessions(snap) => store.restore_snapshot(snap),
        UndoOp::AddedRule(id) => store.delete_rule(id),
        UndoOp::DeletedRule(rule) => store.restore_rule(rule),
        UndoOp::DeletedTag { id, at } => store.restore_tag(id, *at),
    }
}

/// Düzenlemeyi geri alır (en son yapılan önce).
#[tauri::command]
pub async fn undo(app: AppHandle, id: u64) -> CmdResult<()> {
    let ops = app
        .state::<UndoLog>()
        .take(id)
        .ok_or("Bu değişiklik artık geri alınamıyor.")?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    for op in ops.iter().rev() {
        apply(&store, op).map_err(err)?;
    }
    Ok(())
}

fn range(start: &str, days: u32) -> CmdResult<(DateTime<Utc>, DateTime<Utc>)> {
    let first = NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(err)?;
    let from = local_midnight(first);
    let to = local_midnight(first + Days::new(days.clamp(1, 92).into())).min(Utc::now());
    Ok((from, to.max(from)))
}

/// `start` gününden itibaren `days` günün atanmamış süresi.
#[tauri::command]
pub async fn get_unassigned(app: AppHandle, start: String, days: u32) -> CmdResult<Unassigned> {
    let (from, to) = range(&start, days)?;
    lock(&app.state::<Shared>().store)
        .unassigned(from, to)
        .map_err(err)
}

/// Bir grubu (ya da içindeki bir başlığı) projeye atar; `rule` verilirse önce kural eklenir
/// (kural geçmişe de uygulanır; kurala uymayan oturumlar elle atanır).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn assign_unassigned(
    app: AppHandle,
    start: String,
    days: u32,
    key: String,
    title: Option<String>,
    project_id: Option<String>,
    rule: Option<(RuleField, String)>,
) -> CmdResult<Edited> {
    let (from, to) = range(&start, days)?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let mut ops = Vec::new();
    if let (Some(project), Some((field, pattern))) = (&project_id, rule) {
        let id = uuid::Uuid::new_v4().to_string();
        store
            .upsert_rule(&Rule {
                id: id.clone(),
                tag_id: project.clone(),
                field,
                pattern,
            })
            .map_err(err)?;
        ops.push(UndoOp::AddedRule(id));
    }
    let ids = store
        .unassigned_sessions(from, to, &key, title.as_deref())
        .map_err(err)?;
    let snap = store.snapshot_sessions(&ids).map_err(err)?;
    let changed = store
        .set_project_for(&ids, project_id.as_deref())
        .map_err(err)?;
    ops.push(UndoOp::Sessions(snap));
    drop(store);
    Ok(Edited {
        changed,
        undo: record(&app, ops),
    })
}

/// Grubu atanmamış listesinde gösterme ya da yeniden göster.
#[tauri::command]
pub async fn ignore_unassigned(app: AppHandle, key: String, ignored: bool) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .set_unassigned_ignored(&key, ignored)
        .map_err(err)
}

/// Yoksayılan gruplar.
#[tauri::command]
pub async fn ignored_unassigned(app: AppHandle) -> CmdResult<Vec<String>> {
    lock(&app.state::<Shared>().store)
        .ignored_unassigned()
        .map_err(err)
}

/// Kural eklenseydi son 30 günde ne değişirdi?
#[tauri::command]
pub async fn preview_rule(
    app: AppHandle,
    tag_id: String,
    field: RuleField,
    pattern: String,
) -> CmdResult<RulePreview> {
    let rule = Rule {
        id: String::new(),
        tag_id,
        field,
        pattern,
    };
    lock(&app.state::<Shared>().store)
        .preview_rule(&rule, Utc::now(), PREVIEW_DAYS)
        .map_err(err)
}
