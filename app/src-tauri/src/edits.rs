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
use tracky_core::learn::RuleCandidate;
use tracky_core::store::EditSnapshot;
use tracky_core::{NO_PROJECT, Rule, RuleField, Store};

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
    DeletedTag {
        id: String,
        at: DateTime<Utc>,
    },
    /// Projenin önceki arşiv anı (`None`: arşivde değildi).
    Archived {
        id: String,
        previous: Option<DateTime<Utc>>,
    },
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

    /// Geri alma başarısız olduysa (hiçbiri uygulanmadı) işlemleri aynı numarayla yerine koyar;
    /// kullanıcı "Geri al"ı yeniden deneyebilir.
    fn put_back(&self, id: u64, ops: Vec<UndoOp>) {
        let mut log = lock(&self.0);
        let i = log.list.partition_point(|(n, _)| *n < id);
        log.list.insert(i, (id, ops));
        if log.list.len() > MAX_UNDO {
            log.list.remove(0);
        }
    }
}

/// Değişiklik sayısı ve geri alma numarası.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edited {
    pub changed: usize,
    pub undo: u64,
    /// Elle atamadan sonra: atanan süre bir alışkanlığa dönüştüyse önerilen kural
    /// (bildirimde "Kural yap").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<RuleCandidate>,
}

impl Edited {
    pub fn new(changed: usize, undo: u64) -> Self {
        Self {
            changed,
            undo,
            suggestion: None,
        }
    }
}

/// Elle `project_id` projesine atamadan sonra önerilecek kural (`[from, to)` içinde, `ids`
/// verilirse yalnızca o oturumlar). Öneri isteğe bağlıdır: hesaplanamazsa atama yine başarılı.
pub fn rule_suggestion_after(
    app: &AppHandle,
    project_id: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    ids: Option<&[uuid::Uuid]>,
) -> Option<RuleCandidate> {
    let project = project_id.filter(|p| *p != NO_PROJECT)?;
    lock(&app.state::<Shared>().store)
        .rule_suggestion_after(Utc::now(), project, from, to, ids)
        .ok()
        .flatten()
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
        UndoOp::Archived { id, previous } => store.set_tag_archived_at(id, *previous),
    }
}

/// Düzenlemeyi geri alır (en son yapılan önce).
#[tauri::command]
pub async fn undo(app: AppHandle, id: u64) -> CmdResult<()> {
    let log = app.state::<UndoLog>();
    let ops = log.take(id).ok_or("Bu değişiklik artık geri alınamıyor.")?;
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    undo_ops(&store, &ops).map_err(|e| {
        log.put_back(id, ops);
        err(e)
    })
}

/// İşlemleri sondan başa, tek işlemde (transaction) uygular: biri başarısız olursa hiçbiri
/// kalmaz.
fn undo_ops(store: &Store, ops: &[UndoOp]) -> tracky_core::store::Result<()> {
    store.atomic(|store| ops.iter().rev().try_for_each(|op| apply(store, op)))
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
    let with_rule = rule.is_some();
    let (changed, ops, ids) = store
        .atomic(|store| {
            assign(
                store,
                from,
                to,
                &key,
                title.as_deref(),
                project_id.as_deref(),
                rule,
            )
        })
        .map_err(err)?;
    drop(store);
    let mut edited = Edited::new(changed, record(&app, ops));
    // Kuralla atandıysa öneri gereksiz.
    if !with_rule && changed > 0 {
        edited.suggestion =
            rule_suggestion_after(&app, project_id.as_deref(), from, to, Some(&ids));
    }
    Ok(edited)
}

/// [`assign_unassigned`]'ın depo tarafı; çağıran tek işlemde çalıştırır (kural eklenip
/// oturumlar atanamazsa kural da kalmaz).
fn assign(
    store: &Store,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    key: &str,
    title: Option<&str>,
    project_id: Option<&str>,
    rule: Option<(RuleField, String)>,
) -> tracky_core::store::Result<(usize, Vec<UndoOp>, Vec<uuid::Uuid>)> {
    let mut ops = Vec::new();
    if let (Some(project), Some((field, pattern))) = (project_id, rule) {
        let id = uuid::Uuid::new_v4().to_string();
        store.upsert_rule(&Rule {
            id: id.clone(),
            tag_id: project.to_string(),
            field,
            pattern,
        })?;
        ops.push(UndoOp::AddedRule(id));
    }
    let ids = store.unassigned_sessions(from, to, key, title)?;
    let snap = store.snapshot_sessions(&ids)?;
    let changed = store.set_project_for(&ids, project_id)?;
    ops.push(UndoOp::Sessions(snap));
    Ok((changed, ops, ids))
}

/// Takvim bloğundaki bir pencereyi (uygulama + başlık, `[from, to)` içinde) projeye atar;
/// `None` elle atamayı kaldırıp kurallara bırakır. Geri alınabilir.
#[tauri::command]
pub async fn assign_window(
    app: AppHandle,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    app_id: String,
    title: String,
    project_id: Option<String>,
) -> CmdResult<Edited> {
    let shared = app.state::<Shared>();
    let store = lock(&shared.store);
    let (changed, ops, ids) = store
        .atomic(|store| {
            let ids = store.window_sessions(from, to, &app_id, &title)?;
            let snap = store.snapshot_sessions(&ids)?;
            let changed = store.set_project_for(&ids, project_id.as_deref())?;
            Ok((changed, vec![UndoOp::Sessions(snap)], ids))
        })
        .map_err(err)?;
    drop(store);
    let mut edited = Edited::new(changed, record(&app, ops));
    if changed > 0 {
        edited.suggestion =
            rule_suggestion_after(&app, project_id.as_deref(), from, to, Some(&ids));
    }
    Ok(edited)
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

/// Son 30 günün elle atamalarından öğrenilen kural önerileri (Gözden geçir).
#[tauri::command]
pub async fn rule_suggestions(app: AppHandle) -> CmdResult<Vec<RuleCandidate>> {
    lock(&app.state::<Shared>().store)
        .rule_suggestions(Utc::now())
        .map_err(err)
}

/// Kural önerisini bir daha gösterme.
#[tauri::command]
pub async fn dismiss_rule_suggestion(app: AppHandle, key: String) -> CmdResult<()> {
    lock(&app.state::<Shared>().store)
        .dismiss_rule_suggestion(&key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tracky_core::{Session, Tag, TagKind};

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + min * 60, 0).unwrap()
    }

    fn tag(store: &Store, kind: TagKind) -> String {
        let tag = Tag {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            name: "kum".into(),
            color: 1,
        };
        store.upsert_tag(&tag, 0).unwrap();
        tag.id
    }

    fn session(title: &str, from: i64, to: i64) -> Session {
        Session {
            id: uuid::Uuid::new_v4(),
            app_id: "com.apple.Safari".into(),
            app_name: "Safari".into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
        }
    }

    fn projects(store: &Store) -> Vec<Option<String>> {
        let mut v: Vec<_> = store
            .sessions_between(t(-10), t(100))
            .unwrap()
            .into_iter()
            .map(|s| s.project_id)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn failed_undo_changes_nothing_and_can_be_retried() {
        let store = Store::open_in_memory().unwrap();
        let p = tag(&store, TagKind::Project);
        let rule = store.rules().unwrap()[0].clone();
        store.upsert_session(&session("Plan", 0, 30)).unwrap();
        let ids: Vec<_> = store
            .sessions_between(t(0), t(30))
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        let snap = store.snapshot_sessions(&ids).unwrap();
        store.set_project_for(&ids, Some(&p)).unwrap();

        // Sondan başa: kural silinir, oturum geri döner, sonra var olmayan etiketin geri
        // getirilmesi başarısız olur. Hiçbiri kalmamalı.
        let ops = vec![
            UndoOp::DeletedTag {
                id: "yok".into(),
                at: Utc::now(),
            },
            UndoOp::Sessions(snap),
            UndoOp::AddedRule(rule.id.clone()),
        ];
        assert!(undo_ops(&store, &ops).is_err());
        assert!(store.rules().unwrap().iter().any(|r| r.id == rule.id));
        assert_eq!(projects(&store), vec![Some(p.clone())]);

        // Sorunlu işlem olmadan yeniden denenince hepsi uygulanır.
        assert!(undo_ops(&store, &ops[1..]).is_ok());
        assert!(store.rules().unwrap().iter().all(|r| r.id != rule.id));
        assert_eq!(projects(&store), vec![None]);

        let log = UndoLog::default();
        let (older, newer) = (log.push(Vec::new()), log.push(Vec::new()));
        let failed = log.push(Vec::new());
        let taken = log.take(failed).unwrap();
        log.put_back(failed, taken);
        assert!(log.take(failed).is_some());
        assert!(log.take(older).is_some() && log.take(newer).is_some());
    }

    #[test]
    fn failed_assign_does_not_leave_the_rule() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("Plan", 0, 30)).unwrap();
        let rules = store.rules().unwrap().len();
        // Kategoriye kural eklenebilir ama oturumlara proje olarak verilemez: atama başarısız
        // olur, kural da geri alınmalı.
        let category = tag(&store, TagKind::Category);
        let rule = Some((RuleField::Title, "LOY-".to_string()));
        let out = store.atomic(|s| {
            assign(
                s,
                t(0),
                t(60),
                "app:com.apple.Safari",
                None,
                Some(&category),
                rule.clone(),
            )
        });
        assert!(out.is_err());
        assert_eq!(store.rules().unwrap().len(), rules);

        let p = tag(&store, TagKind::Project);
        let (changed, ops, _) = store
            .atomic(|s| assign(s, t(0), t(60), "app:com.apple.Safari", None, Some(&p), rule))
            .unwrap();
        assert_eq!(changed, 1);
        assert_eq!(store.rules().unwrap().len(), rules + 1);
        assert_eq!(projects(&store), vec![Some(p)]);
        undo_ops(&store, &ops).unwrap();
        assert_eq!(store.rules().unwrap().len(), rules);
        assert_eq!(projects(&store), vec![None]);
    }

    #[test]
    fn archiving_can_be_undone() {
        let store = Store::open_in_memory().unwrap();
        let kum = store.accept_project_suggestion("Kum").unwrap();
        let snap = store.archive_project(&kum.id).unwrap();
        assert!(store.archived_projects().unwrap().contains(&kum.id));
        // archive_project komutunun kaydettiği işlemler.
        let ops = vec![
            UndoOp::Archived {
                id: kum.id.clone(),
                previous: None,
            },
            UndoOp::Sessions(snap),
        ];
        assert!(undo_ops(&store, &ops).is_ok());
        assert!(store.archived_projects().unwrap().is_empty());
        assert!(store.rules().unwrap().iter().any(|r| r.tag_id == kum.id));

        // Arşivden çıkarmanın geri alınması önceki arşiv anına döner.
        store.archive_project(&kum.id).unwrap();
        let previous = store.unarchive_project(&kum.id).unwrap();
        assert!(previous.is_some());
        let ops = vec![UndoOp::Archived {
            id: kum.id.clone(),
            previous,
        }];
        assert!(undo_ops(&store, &ops).is_ok());
        assert_eq!(store.tag_extras().unwrap()[&kum.id].archived_at, previous);
    }
}
