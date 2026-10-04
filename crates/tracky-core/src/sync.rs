//! Cihazlar arası senkronizasyon: yerel değişiklikleri gönder, uzaktakileri çek.
//!
//! Kurallar:
//! - Her satır UUID ile tanımlıdır; `updated_at` son değişiklik anıdır.
//! - Çakışmada daha yeni `updated_at` kazanır (uzak tarafta da, yerelde de).
//! - Silme yumuşaktır (`deleted_at`), böylece silinmeler de senkronize olur.
//! - `synced_at < updated_at` olan yerel satırlar gönderilmeyi bekler.
//! - Uzak taraf her satıra sunucu saatiyle `server_updated_at` verir; çekme
//!   bu imleçten sonrasını ister.
//! - Gönderilen satırlar `writer` (bu açılışın kimliği) taşır; çekme, sunucudaki
//!   son sürümü bu kurulumun yazdığı satırları atlar ve her şey çekildiyse sonunda imleci
//!   çekmeden önceki en yeni sunucu zamanına taşır (kendi satırları çekilmese de imleç
//!   onları geçer).
//!   Sütunu olmayan eski şemada (0003 öncesi) filtre olmadan çalışılır.
//!
//! Ağ çağrıları sırasında depo kilidi tutulmaz; takip sürerken senkronizasyon
//! yapılabilir.

use std::sync::Mutex;

use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value as SqlValue};
use serde_json::{Map, Value};

use crate::store::Store;

/// Uzak depo (Supabase ya da testlerde bellek içi sahte).
pub trait Remote {
    /// Satırları upsert eder (uzak taraf eski sürümleri reddeder).
    fn push(&mut self, table: &str, rows: &[Value]) -> Result<(), String>;
    /// `since`'den (hariç) sonra değişen satırlar, `server_updated_at`'e göre artan.
    /// `skip_writer` verilirse son sürümünü o cihazın yazdığı satırlar gelmez.
    fn pull(
        &mut self,
        table: &str,
        since: Option<&str>,
        skip_writer: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String>;
    /// `since`'den (hariç) sonra değişen satırların en yeni `server_updated_at`'i.
    fn latest(&mut self, table: &str, since: Option<&str>) -> Result<Option<String>, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Col {
    Text,
    OptText,
    Int,
    Time,
    OptTime,
}

struct Table {
    name: &'static str,
    /// İlk sütun `id`, `updated_at` her tabloda var.
    cols: &'static [(&'static str, Col)],
}

/// Sıra önemli: kurallar etiketlere başvurur, önce etiketler uygulanır. Etiketlerin müşterisi
/// (`client_id`) yabancı anahtar değildir; müşteri sonra gelse de etiket uygulanır.
const TABLES: &[Table] = &[
    Table {
        name: "clients",
        cols: &[
            ("id", Col::Text),
            ("name", Col::Text),
            ("position", Col::Int),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
    },
    Table {
        name: "tags",
        cols: &[
            ("id", Col::Text),
            ("kind", Col::Text),
            ("name", Col::Text),
            ("color", Col::Int),
            ("position", Col::Int),
            ("client_id", Col::OptText),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
    },
    Table {
        name: "rules",
        cols: &[
            ("id", Col::Text),
            ("tag_id", Col::Text),
            ("field", Col::Text),
            ("pattern", Col::Text),
            ("position", Col::Int),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
    },
    Table {
        name: "sessions",
        cols: &[
            ("id", Col::Text),
            ("device_id", Col::Text),
            ("app_id", Col::Text),
            ("app_name", Col::Text),
            ("title", Col::Text),
            ("url", Col::OptText),
            ("domain", Col::OptText),
            ("started_at", Col::Time),
            ("ended_at", Col::Time),
            ("category_id", Col::OptText),
            ("project_id", Col::OptText),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
    },
];

const PUSH_BATCH: usize = 500;
const PULL_BATCH: usize = 1000;
/// Bir çalıştırmada en fazla bu kadar parti (sonsuz döngüye karşı).
const MAX_BATCHES: usize = 200;
/// Eşzamanlı işlemlerde sunucu saatinin geride kalan satırlarını kaçırmamak için
/// her çalıştırmada imleç bu kadar geriden başlar (yeniden uygulamak zararsız).
const CURSOR_OVERLAP_SECS: i64 = 5;
/// Satırı sunucuda son yazan cihazın sütunu (supabase/migrations/0003).
const WRITER: &str = "writer";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SyncSummary {
    pub pushed: usize,
    pub pulled: usize,
    /// Bu sürümün kabul etmediği (geçersiz ya da kısıtı ihlal eden) uzak satırlar.
    pub skipped: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("sunucu: {0}")]
    Remote(String),
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error("veritabanı: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("geçersiz uzak satır: {0}")]
    Invalid(String),
}

/// Tüm tabloları gönderir ve çeker. `user_id` gönderilen satırlara eklenir.
pub fn run(
    store: &Mutex<Store>,
    remote: &mut dyn Remote,
    user_id: &str,
) -> Result<SyncSummary, SyncError> {
    // Bir tablodaki hata diğerlerini durdurmaz; ilk hata sonunda bildirilir.
    let mut summary = SyncSummary::default();
    let mut first_error = None;
    let mut tags_failed = false;
    let device = lock(store).instance_id().to_string();
    // Sunucuda `writer` sütunu yoksa ilk hatada kapanır.
    let mut writer = Some(device.as_str());
    for table in TABLES {
        // Etiketleri gönderilemeyen kurallar sunucuya etiketsiz düşer ve diğer
        // cihazlarda çekimi tıkar; bir sonraki çalıştırmaya bırak.
        if table.name == "rules" && tags_failed {
            continue;
        }
        match push_table(store, remote, table, user_id, &mut writer) {
            Ok(n) => summary.pushed += n,
            Err(e) => {
                tags_failed |= table.name == "tags";
                first_error.get_or_insert(e);
            }
        }
    }
    for table in TABLES {
        let mut result = pull_table(store, remote, table, &mut writer);
        // Etiketler çekildikten sonra başka cihaz yeni etiket + kural eklemiş olabilir:
        // kuralın etiketi yerelde yoksa etiketleri yeniden çekip bir kez daha dene.
        if table.name == "rules" && result.as_ref().is_err_and(is_foreign_key_error) {
            let tags = TABLES
                .iter()
                .find(|t| t.name == "tags")
                .expect("etiket tablosu");
            let _ = pull_table(store, remote, tags, &mut writer);
            result = pull_table(store, remote, table, &mut writer);
        }
        match result {
            Ok((n, skipped)) => {
                summary.pulled += n;
                summary.skipped += skipped;
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(summary),
    }
}

/// Satırı atlamak eşitlemeyi kurtarır mı? Ayrıştırılamayan ya da CHECK gibi bir kısıtı
/// ihlal eden satır (örn. sonraki bir sürümün yeni değeri) tekrar denense de uygulanamaz;
/// atlanmazsa imleç hiç ilerlemez ve bu cihazın eşitlemesi kalıcı olarak takılır.
/// Yabancı anahtar hatası atlanmaz: etiketler yeniden çekilince çözülebilir.
fn is_unappliable(e: &SyncError) -> bool {
    match e {
        SyncError::Invalid(_) => true,
        SyncError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) => {
            f.code == rusqlite::ErrorCode::ConstraintViolation
                && f.extended_code != rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
        }
        _ => false,
    }
}

fn is_foreign_key_error(e: &SyncError) -> bool {
    matches!(
        e,
        SyncError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
            if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

/// Sunucu şeması 0003'ten eskiyse `writer` sütunu bulunamaz.
fn missing_writer(e: &str) -> bool {
    e.contains(WRITER)
}

fn lock(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

fn push_table(
    store: &Mutex<Store>,
    remote: &mut dyn Remote,
    table: &Table,
    user_id: &str,
    writer: &mut Option<&str>,
) -> Result<usize, SyncError> {
    let mut total = 0;
    for _ in 0..MAX_BATCHES {
        let rows = unsynced(&lock(store), table, PUSH_BATCH)?;
        if rows.is_empty() {
            break;
        }
        let payload = |writer: Option<&str>| -> Vec<Value> {
            rows.iter()
                .map(|(row, _, _)| {
                    let mut row = row.clone();
                    row.insert("user_id".into(), Value::String(user_id.into()));
                    if let Some(w) = writer {
                        row.insert(WRITER.into(), Value::String(w.into()));
                    }
                    Value::Object(row)
                })
                .collect()
        };
        let mut result = remote.push(table.name, &payload(*writer));
        if writer.is_some() && result.as_ref().is_err_and(|e| missing_writer(e)) {
            *writer = None;
            result = remote.push(table.name, &payload(None));
        }
        result.map_err(|e| {
            // 0.2 ile eklenen sütun sunucuda yoksa kullanıcıya ne yapacağını söyle.
            if e.contains("category_id") {
                SyncError::Remote(format!(
                    "Supabase şeması güncel değil: supabase/migrations/0002_session_category.sql \
                     dosyasını SQL Editor'da çalıştırın ({e})"
                ))
            } else if e.contains("project_id") {
                SyncError::Remote(format!(
                    "Supabase şeması güncel değil: supabase/migrations/0004_session_project.sql \
                     dosyasını SQL Editor'da çalıştırın ({e})"
                ))
            } else {
                SyncError::Remote(e)
            }
        })?;
        // Gönderilen sürüm işaretlenir; arada yerelde güncellenen satır kirli kalır.
        let marked = mark_synced(&lock(store), table, &rows)?;
        total += rows.len();
        if rows.len() < PUSH_BATCH || marked == 0 {
            break;
        }
    }
    Ok(total)
}

fn pull_table(
    store: &Mutex<Store>,
    remote: &mut dyn Remote,
    table: &Table,
    writer: &mut Option<&str>,
) -> Result<(usize, usize), SyncError> {
    pull_pages(store, remote, table, writer, MAX_BATCHES)
}

fn pull_pages(
    store: &Mutex<Store>,
    remote: &mut dyn Remote,
    table: &Table,
    writer: &mut Option<&str>,
    max_batches: usize,
) -> Result<(usize, usize), SyncError> {
    let key = format!("sync_cursor:{}", table.name);
    let mut cursor: Option<String> = lock(store).setting(&key)?;
    // Kendi satırları filtrelenirse imleç onların gerisinde kalır; her şey çekilince en yeni
    // sunucu zamanına taşınır. Bu zaman çekmeden *önce* alınır: çekme sürerken yazılan bir
    // satır, imleç onun ötesine atlayıp kaçırılmasın.
    let latest = match writer {
        Some(_) => remote
            .latest(table.name, cursor.as_deref())
            .map_err(SyncError::Remote)?,
        None => None,
    };
    let mut since = cursor.as_deref().map(overlap);
    let (mut total, mut skipped) = (0, 0);
    let mut drained = false;
    for _ in 0..max_batches {
        let mut result = remote.pull(table.name, since.as_deref(), *writer, PULL_BATCH);
        if writer.is_some() && result.as_ref().is_err_and(|e| missing_writer(e)) {
            *writer = None;
            result = remote.pull(table.name, since.as_deref(), None, PULL_BATCH);
        }
        let rows = result.map_err(SyncError::Remote)?;
        if rows.is_empty() {
            drained = true;
            break;
        }
        let newest = rows
            .iter()
            .filter_map(|r| r.get("server_updated_at")?.as_str())
            .max_by_key(|s| parse_time(s).ok())
            .map(str::to_string);
        {
            let store = lock(store);
            // Tek işlem: bin satır için bin ayrı diske yazma yerine bir tane.
            let tx = store.conn().unchecked_transaction()?;
            for row in &rows {
                match apply_remote(&store, table, row) {
                    Ok(n) => total += n,
                    Err(e) if is_unappliable(&e) => skipped += 1,
                    Err(e) => return Err(e),
                }
            }
            if let Some(newest) = &newest
                && cursor.as_deref().is_none_or(|c| later(newest, c))
            {
                store.save_setting(&key, newest)?;
                cursor = Some(newest.clone());
            }
            // Hata olursa geri alınır; imleç de ilerlemez, satırlar tekrar çekilir.
            tx.commit()?;
        }
        if rows.len() < PULL_BATCH {
            drained = true;
            break;
        }
        let Some(newest) = newest else {
            break;
        };
        // Sayfa sınırında aynı sunucu zamanlı satırlar bölünebilir: sonraki sayfa sınırın
        // kendisini de kapsar (zaten uygulanan satırlar yeniden uygulanınca değişmez).
        since = Some(just_before(&newest));
    }
    // Yalnızca her şey çekildiyse (son sayfa kısa ya da boş) imleç baştaki en yeni zamana
    // atlar. Parti sınırına takıldıysa imleç son çekilen sayfada kalır; kalanı sonraki
    // çalıştırma çeker (yoksa çekilmemiş satırlar kalıcı olarak atlanırdı).
    if drained
        && let Some(latest) = latest
        && cursor.as_deref().is_none_or(|c| later(&latest, c))
    {
        lock(store).save_setting(&key, &latest)?;
    }
    Ok((total, skipped))
}

/// Gönderilmeyi bekleyen satır: (JSON, id, updated_at ms).
type Pending = (Map<String, Value>, String, i64);

fn unsynced(store: &Store, table: &Table, limit: usize) -> Result<Vec<Pending>, SyncError> {
    let names: Vec<&str> = table.cols.iter().map(|c| c.0).collect();
    let sql = format!(
        "SELECT {} FROM {} WHERE synced_at IS NULL OR synced_at < updated_at
         ORDER BY updated_at LIMIT ?1",
        names.join(", "),
        table.name
    );
    let mut stmt = store.conn().prepare(&sql)?;
    let rows = stmt.query_map([limit as i64], |r| {
        let mut obj = Map::new();
        let mut id = String::new();
        let mut updated = 0;
        for (i, (name, col)) in table.cols.iter().enumerate() {
            let value = match col {
                Col::Text => Value::String(r.get(i)?),
                Col::OptText => r
                    .get::<_, Option<String>>(i)?
                    .map_or(Value::Null, Value::String),
                Col::Int => Value::from(r.get::<_, i64>(i)?),
                Col::Time => Value::String(iso(r.get(i)?)),
                Col::OptTime => r
                    .get::<_, Option<i64>>(i)?
                    .map_or(Value::Null, |v| Value::String(iso(v))),
            };
            if *name == "id" {
                id = r.get(i)?;
            }
            if *name == "updated_at" {
                updated = r.get(i)?;
            }
            obj.insert(name.to_string(), value);
        }
        Ok((obj, id, updated))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn mark_synced(store: &Store, table: &Table, rows: &[Pending]) -> Result<usize, SyncError> {
    let sql = format!(
        "UPDATE {} SET synced_at = ?2 WHERE id = ?1 AND updated_at = ?2",
        table.name
    );
    let tx = store.conn().unchecked_transaction()?;
    let mut marked = 0;
    {
        let mut stmt = tx.prepare(&sql)?;
        for (_, id, updated) in rows {
            marked += stmt.execute(rusqlite::params![id, updated])?;
        }
    }
    tx.commit()?;
    Ok(marked)
}

/// Uzak satırı yerelde uygular; yerel sürüm daha yeniyse dokunmaz. Değişen satır sayısı.
fn apply_remote(store: &Store, table: &Table, row: &Value) -> Result<usize, SyncError> {
    let obj = row
        .as_object()
        .ok_or_else(|| SyncError::Invalid(row.to_string()))?;
    let mut values = Vec::with_capacity(table.cols.len() + 1);
    let mut updated = 0;
    for (name, col) in table.cols {
        let v = obj.get(*name).unwrap_or(&Value::Null);
        let bad = || SyncError::Invalid(format!("{}.{name}: {v}", table.name));
        let sql = match (col, v) {
            (Col::Text, Value::String(s)) => SqlValue::Text(s.clone()),
            (Col::OptText, Value::String(s)) => SqlValue::Text(s.clone()),
            (Col::OptText | Col::OptTime, Value::Null) => SqlValue::Null,
            (Col::Int, Value::Number(n)) => SqlValue::Integer(n.as_i64().ok_or_else(bad)?),
            (Col::Time | Col::OptTime, Value::String(s)) => {
                SqlValue::Integer(parse_time(s).map_err(|_| bad())?.timestamp_millis())
            }
            _ => return Err(bad()),
        };
        if *name == "updated_at"
            && let SqlValue::Integer(ms) = sql
        {
            updated = ms;
        }
        values.push(sql);
    }
    values.push(SqlValue::Integer(updated)); // synced_at

    let names: Vec<&str> = table.cols.iter().map(|c| c.0).collect();
    let placeholders: Vec<String> = (1..=names.len() + 1).map(|i| format!("?{i}")).collect();
    let updates: Vec<String> = names
        .iter()
        .skip(1)
        .chain(std::iter::once(&"synced_at"))
        .map(|n| format!("{n} = excluded.{n}"))
        .collect();
    let sql = format!(
        "INSERT INTO {t} ({cols}, synced_at) VALUES ({ph})
         ON CONFLICT (id) DO UPDATE SET {up}
         WHERE excluded.updated_at > {t}.updated_at",
        t = table.name,
        cols = names.join(", "),
        ph = placeholders.join(", "),
        up = updates.join(", "),
    );
    Ok(store.conn().execute(&sql, params_from_iter(values))?)
}

fn iso(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_time(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc))
}

fn later(a: &str, b: &str) -> bool {
    match (parse_time(a), parse_time(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a > b,
    }
}

/// Sunucu zamanından bir mikrosaniye önce (sunucu zamanları mikrosaniye çözünürlüklü).
fn just_before(at: &str) -> String {
    parse_time(at)
        .map(|t| (t - Duration::microseconds(1)).to_rfc3339_opts(SecondsFormat::Micros, true))
        .unwrap_or_else(|_| at.to_string())
}

fn overlap(cursor: &str) -> String {
    parse_time(cursor)
        .map(|t| {
            (t - Duration::seconds(CURSOR_OVERLAP_SECS))
                .to_rfc3339_opts(SecondsFormat::Micros, true)
        })
        .unwrap_or_else(|_| cursor.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{RuleField, Tag};
    use crate::model::Session;
    use std::collections::HashMap;
    use uuid::Uuid;

    /// Supabase davranışını taklit eder: son yazan kazanır, sunucu imleci atar.
    #[derive(Default)]
    struct FakeRemote {
        rows: HashMap<String, HashMap<String, Value>>,
        clock: i64,
        pushes: usize,
        /// 0003 öncesi şema: `writer` sütunu yok.
        legacy: bool,
        /// Bir sonraki çekmenin hemen ardından (başka cihazlarca) yazılacak satırlar.
        write_after_pull: Vec<(String, Value)>,
    }

    const NO_WRITER: &str = "Could not find the 'writer' column in the schema cache";

    impl Remote for FakeRemote {
        fn push(&mut self, table: &str, rows: &[Value]) -> Result<(), String> {
            self.pushes += 1;
            if self.legacy && rows.iter().any(|r| r.get(WRITER).is_some()) {
                return Err(NO_WRITER.into());
            }
            let t = self.rows.entry(table.into()).or_default();
            for row in rows {
                let id = row["id"].as_str().unwrap().to_string();
                let newer = |old: &Value| {
                    parse_time(row["updated_at"].as_str().unwrap()).unwrap()
                        >= parse_time(old["updated_at"].as_str().unwrap()).unwrap()
                };
                if t.get(&id).is_none_or(newer) {
                    self.clock += 1;
                    let mut row = row.clone();
                    row["server_updated_at"] =
                        Value::String(iso(1_700_000_000_000 + self.clock * 10_000));
                    t.insert(id, row);
                }
            }
            Ok(())
        }

        fn pull(
            &mut self,
            table: &str,
            since: Option<&str>,
            skip_writer: Option<&str>,
            limit: usize,
        ) -> Result<Vec<Value>, String> {
            if self.legacy && skip_writer.is_some() {
                return Err(NO_WRITER.into());
            }
            let since = since.map(|s| parse_time(s).unwrap());
            let mut rows: Vec<Value> = self
                .rows
                .get(table)
                .map(|t| t.values().cloned().collect())
                .unwrap_or_default();
            rows.retain(|r| {
                skip_writer.is_none_or(|w| r.get(WRITER).and_then(Value::as_str) != Some(w))
            });
            rows.retain(|r| {
                since.is_none_or(|s| {
                    parse_time(r["server_updated_at"].as_str().unwrap()).unwrap() > s
                })
            });
            rows.sort_by_key(|r| r["server_updated_at"].as_str().unwrap().to_string());
            rows.truncate(limit);
            for (table, row) in std::mem::take(&mut self.write_after_pull) {
                self.push(&table, &[row])?;
            }
            Ok(rows)
        }

        fn latest(&mut self, table: &str, since: Option<&str>) -> Result<Option<String>, String> {
            let since = since.map(|s| parse_time(s).unwrap());
            Ok(self
                .rows
                .get(table)
                .into_iter()
                .flat_map(|t| t.values())
                .filter_map(|r| r["server_updated_at"].as_str())
                .filter(|t| since.is_none_or(|s| parse_time(t).unwrap() > s))
                .max()
                .map(str::to_string))
        }
    }

    fn session(app: &str, secs: i64) -> Session {
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        Session {
            id: Uuid::new_v4(),
            app_id: format!("com.test.{app}"),
            app_name: app.into(),
            title: "t".into(),
            url: None,
            domain: None,
            started_at: t0,
            ended_at: t0 + Duration::seconds(secs),
            category_id: None,
            project_id: None,
        }
    }

    /// Uygulama başına ham (birleştirilmemiş) süre: eşitlenen satırları sayar.
    fn totals(store: &Mutex<Store>) -> Vec<(String, i64)> {
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let mut out: Vec<(String, i64)> = Vec::new();
        for s in lock(store)
            .sessions_between(t0, t0 + Duration::hours(1))
            .unwrap()
        {
            let secs = (s.ended_at - s.started_at).num_seconds();
            match out.iter_mut().find(|(name, _)| *name == s.app_name) {
                Some((_, total)) => *total += secs,
                None => out.push((s.app_name, secs)),
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    #[test]
    fn two_devices_converge() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();

        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        lock(&b).upsert_session(&session("Safari", 30)).unwrap();

        // İlk senkronizasyonda varsayılan kategoriler de gönderilir; aynı kimlikli
        // oldukları için uzakta kopya oluşmaz.
        let first = run(&a, &mut remote, "u1").unwrap();
        assert!(first.pushed > 1);
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
        assert_eq!(remote.rows["tags"].len(), lock(&a).tags().unwrap().len());

        let expected = vec![("Code".to_string(), 60), ("Safari".to_string(), 30)];
        assert_eq!(totals(&a), expected);
        assert_eq!(totals(&b), expected);

        // Değişiklik yoksa ikinci çalıştırma bir şey göndermez.
        let pushes = remote.pushes;
        assert_eq!(run(&a, &mut remote, "u1").unwrap().pushed, 0);
        assert_eq!(remote.pushes, pushes);
    }

    #[test]
    fn edits_and_deletes_propagate_and_newer_wins() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();

        // A bir kategoriyi yeniden adlandırır, B bir kuralı siler.
        let tag: Tag = lock(&a).tags().unwrap()[0].clone();
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&a)
            .upsert_tag(
                &Tag {
                    name: "Kod".into(),
                    ..tag.clone()
                },
                0,
            )
            .unwrap();
        let rule = lock(&b)
            .rules()
            .unwrap()
            .into_iter()
            .find(|r| r.field == RuleField::Title)
            .unwrap();
        lock(&b).delete_rule(&rule.id).unwrap();

        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();

        for store in [&a, &b] {
            let s = lock(store);
            assert_eq!(
                s.tags()
                    .unwrap()
                    .iter()
                    .find(|t| t.id == tag.id)
                    .unwrap()
                    .name,
                "Kod"
            );
            assert!(s.rules().unwrap().iter().all(|r| r.id != rule.id));
        }

        // Eski bir sürüm yeniyi ezemez: B'nin eski adı uzaktan geri gelmez.
        let row = remote.rows["tags"][&tag.id].clone();
        assert_eq!(row["name"], "Kod");
    }

    #[test]
    fn late_device_seed_does_not_override_edits() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let tag: Tag = lock(&a).tags().unwrap()[0].clone();
        lock(&a)
            .upsert_tag(
                &Tag {
                    name: "Kod".into(),
                    ..tag.clone()
                },
                0,
            )
            .unwrap();
        let rule = lock(&a).rules().unwrap()[0].clone();
        lock(&a).delete_rule(&rule.id).unwrap();
        run(&a, &mut remote, "u1").unwrap();

        // B daha sonra kurulur (tohumu daha yeni saatli olsa da epoch'tur).
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = Mutex::new(Store::open_in_memory().unwrap());
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();

        for store in [&a, &b] {
            let s = lock(store);
            let name = s
                .tags()
                .unwrap()
                .into_iter()
                .find(|t| t.id == tag.id)
                .unwrap()
                .name;
            assert_eq!(name, "Kod");
            assert!(s.rules().unwrap().iter().all(|r| r.id != rule.id));
        }
    }

    #[test]
    fn reset_sync_state_resends_everything() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        let mut first = FakeRemote::default();
        run(&a, &mut first, "u1").unwrap();

        // Başka hesap/proje: sıfırlanmazsa hiçbir şey gönderilmezdi.
        lock(&a).reset_sync_state().unwrap();
        let mut second = FakeRemote::default();
        run(&a, &mut second, "u2").unwrap();
        assert_eq!(second.rows["sessions"].len(), 1);
        assert_eq!(second.rows["tags"].len(), first.rows["tags"].len());
    }

    #[test]
    fn pull_pages_through_large_sets() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        for _ in 0..(PULL_BATCH + 50) {
            lock(&a).upsert_session(&session("Code", 1)).unwrap();
        }
        let pushed = run(&a, &mut remote, "u1").unwrap().pushed;
        assert!(pushed > PULL_BATCH);
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(
            totals(&b),
            vec![("Code".to_string(), (PULL_BATCH + 50) as i64)]
        );
    }

    #[test]
    fn own_rows_are_not_pulled_back() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let first = run(&a, &mut remote, "u1").unwrap();
        assert!(first.pushed > 0);
        assert_eq!(first.pulled, 0);

        // B'nin satırları A'ya gelir, A'nın kendi gönderdikleri gelmez.
        let safari = session("Safari", 30);
        lock(&b).upsert_session(&safari).unwrap();
        run(&b, &mut remote, "u1").unwrap();
        // Varsayılan kategoriler B'de de aynı; yalnızca yeni oturum uygulanır.
        assert_eq!(run(&a, &mut remote, "u1").unwrap().pulled, 1);
        assert_eq!(totals(&a), vec![("Safari".to_string(), 30)]);

        // A, B'nin oturumunu silerse silme B'ye ulaşır, A'ya geri gelmez.
        lock(&a).delete_session(&safari.id).unwrap();
        let deleted = run(&a, &mut remote, "u1").unwrap();
        assert_eq!((deleted.pushed, deleted.pulled), (1, 0));
        assert_eq!(run(&b, &mut remote, "u1").unwrap().pulled, 1);
        assert!(totals(&b).is_empty());
        assert_eq!(run(&a, &mut remote, "u1").unwrap().pulled, 0);
    }

    #[test]
    fn legacy_schema_without_writer_still_syncs() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote {
            legacy: true,
            ..Default::default()
        };
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(totals(&b), vec![("Code".to_string(), 60)]);
        assert!(
            remote.rows["sessions"]
                .values()
                .all(|r| r.get(WRITER).is_none())
        );
    }

    #[test]
    fn invalid_remote_rows_do_not_block_sync() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        run(&a, &mut remote, "u1").unwrap();
        // Gelecekteki bir sürümün yazdığı, bu sürümün kabul etmediği satırlar.
        let mut bad = remote.rows["tags"].values().next().unwrap().clone();
        bad["id"] = Value::String(Uuid::new_v4().to_string());
        bad["color"] = Value::from(42);
        remote.push("tags", &[bad]).unwrap();
        let mut odd = remote.rows["tags"].values().next().unwrap().clone();
        odd["id"] = Value::String(Uuid::new_v4().to_string());
        odd["updated_at"] = Value::String("dün".into());
        remote.push("tags", &[odd]).unwrap_or(());
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        run(&a, &mut remote, "u1").unwrap();

        // B geçersiz satırları atlar, geri kalanını alır ve sonraki çalıştırmalar da işler.
        assert_eq!(run(&b, &mut remote, "u1").unwrap().skipped, 2);
        assert_eq!(totals(&b), vec![("Code".to_string(), 60)]);
        assert!(run(&b, &mut remote, "u1").is_ok());
    }

    #[test]
    fn cursor_moves_past_own_rows() {
        // Tek cihaz: kendi satırları çekilmese de imleç onların ötesine geçmeli; yoksa
        // her eşitleme ilk günden beri yazılan tüm satırları sunucuda yeniden tarar.
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        let newest = remote.rows["sessions"]
            .values()
            .filter_map(|r| r["server_updated_at"].as_str())
            .max()
            .unwrap()
            .to_string();
        let cursor: Option<String> = lock(&a).setting("sync_cursor:sessions").unwrap();
        assert_eq!(cursor.as_deref(), Some(newest.as_str()));
    }

    #[test]
    fn copied_databases_still_sync_with_each_other() {
        // Taşıma Yardımcısı gibi kopyalama: iki kurulum aynı veritabanından açılır.
        let dir = std::env::temp_dir().join(format!("kum-clone-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("a.db");
        drop(Store::open(&original).unwrap());
        std::fs::copy(&original, dir.join("b.db")).unwrap();
        let a = Mutex::new(Store::open(&original).unwrap());
        let b = Mutex::new(Store::open(dir.join("b.db")).unwrap());
        let mut remote = FakeRemote::default();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(totals(&b), vec![("Code".to_string(), 60)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manual_project_syncs_between_devices() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let s = session("Code", 600);
        lock(&a).upsert_session(&s).unwrap();
        let project = lock(&a).accept_project_suggestion("Trumore").unwrap();
        lock(&a)
            .set_project_between(s.started_at, s.ended_at, Some(&project.id))
            .unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        let got = lock(&b).sessions_between(s.started_at, s.ended_at).unwrap();
        assert_eq!(got[0].project_id.as_deref(), Some(project.id.as_str()));
    }

    #[test]
    fn clients_and_project_links_sync_between_devices() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let project = lock(&a).accept_project_suggestion("Trumore").unwrap();
        let togg = crate::classify::Client {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Togg".into(),
        };
        lock(&a).upsert_client(&togg, 0).unwrap();
        lock(&a)
            .set_project_client(&project.id, Some(&togg.id))
            .unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(lock(&b).clients().unwrap(), std::slice::from_ref(&togg));
        assert_eq!(
            lock(&b).project_clients().unwrap().get(&project.id),
            Some(&togg.id)
        );
        // B'de müşteri silinince A'da da silinir, proje müşterisiz kalır.
        lock(&b).delete_client(&togg.id).unwrap();
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
        assert!(lock(&a).clients().unwrap().is_empty());
        assert!(lock(&a).project_clients().unwrap().is_empty());
        assert_eq!(
            lock(&a)
                .tags()
                .unwrap()
                .iter()
                .filter(|t| t.id == project.id)
                .count(),
            1
        );
    }

    #[test]
    fn cursor_stays_when_pull_stops_before_draining() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        for _ in 0..(PULL_BATCH * 2 + 10) {
            lock(&a).upsert_session(&session("Code", 1)).unwrap();
        }
        run(&a, &mut remote, "u1").unwrap();
        // Parti sınırına takılan çekme: imleç en yeni sunucu zamanına atlamamalı.
        let sessions = TABLES.iter().find(|t| t.name == "sessions").unwrap();
        let device = lock(&b).instance_id().to_string();
        let mut writer = Some(device.as_str());
        pull_pages(&b, &mut remote, sessions, &mut writer, 1).unwrap();
        let latest = remote.latest("sessions", None).unwrap().unwrap();
        let cursor: Option<String> = lock(&b).setting("sync_cursor:sessions").unwrap();
        assert!(later(&latest, cursor.as_deref().unwrap()));
        // Sonraki çalıştırma kalanını çeker.
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(
            totals(&b),
            vec![("Code".to_string(), (PULL_BATCH * 2 + 10) as i64)]
        );
    }

    #[test]
    fn rows_written_during_pull_are_not_skipped() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        // B çekerken iki başka cihaz yazar; ikincisi imleç örtüşmesinden daha geç.
        let template = remote.rows["sessions"].values().next().unwrap().clone();
        for (app, writer) in [("Safari", "c"), ("Mail", "d")] {
            let mut row = template.clone();
            row["id"] = Value::String(Uuid::new_v4().to_string());
            row["app_name"] = Value::String(app.into());
            row["writer"] = Value::String(writer.into());
            remote.write_after_pull.push(("sessions".into(), row));
        }
        // Etiketler önce çekilir; satırları oturum çekmesinin ardından yazdırmak için
        // yalnızca oturum tablosu çekilir.
        let sessions = TABLES.iter().find(|t| t.name == "sessions").unwrap();
        let device = lock(&b).instance_id().to_string();
        let mut writer = Some(device.as_str());
        pull_table(&b, &mut remote, sessions, &mut writer).unwrap();
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(
            totals(&b),
            vec![
                ("Code".to_string(), 60),
                ("Mail".to_string(), 60),
                ("Safari".to_string(), 60)
            ]
        );
    }
}
