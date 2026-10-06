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
    Real,
    OptReal,
}

struct Table {
    name: &'static str,
    /// İlk sütun `id`, `updated_at` her tabloda var.
    cols: &'static [(&'static str, Col)],
    /// Sonradan eklenen, eski sunucu şemasında bulunmayabilecek sütunlar (0007: arşiv ve
    /// bütçe). Sunucuda yoksa satırlar onlarsız gönderilir; çekilen satırda yoksa yerel değer
    /// korunur.
    optional: &'static [&'static str],
    /// Yereldeki kimlik sütunu (sunucuda her zaman `id`).
    key: &'static str,
    /// Doluysa yalnızca bu kimlikteki satırlar gönderilir ve uygulanır.
    only: Option<&'static [&'static str]>,
}

/// Cihazlar arasında taşınan ayarlar (settings tablosunun anahtarları). Yeni Mac'te giriş
/// yapınca zaman çizelgeleri, takvim, gizlilik, hedefler, görünüm ve yapay zekâ ayarları da
/// gelir. Cihaza özgü olanlar (oturum jetonları, imleçler, otomatik başlatma, izin
/// kurulumu, duraklatma süresi, bildirim ve yedek işaretleri) eşitlenmez.
pub const SYNCED_SETTINGS: &[&str] = &[
    "privacy",
    "goals",
    "theme",
    "timesheet",
    "timesheet_details",
    "meeting_assignments",
    "calendar_url",
    "ai_details",
    "dismissed_suggestions",
    "dismissed_rule_suggestions",
    "ignored_unassigned",
];

/// Eşitlenen ayarlarda yalnızca bu cihazda kalan alanlar: sunucuya gönderilmez, gelen
/// sürümde yerel değer korunur. Bir Mac'te duraklatmak diğerini duraklatmaz; API anahtarı
/// gizlidir ve sunucuda (sürüm derlemelerinde ortak projede) açık metin durmamalı.
const LOCAL_FIELDS: &[(&str, &[&str])] = &[("privacy", &["paused"]), ("ai_details", &["apiKey"])];

fn local_fields(key: &str) -> &'static [&'static str] {
    LOCAL_FIELDS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(&[], |(_, fields)| fields)
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
            ("budget_days", Col::OptReal),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
        optional: &["budget_days"],
        key: "id",
        only: None,
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
            ("archived_at", Col::OptTime),
            ("budget_days", Col::OptReal),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
        optional: &["archived_at", "budget_days"],
        key: "id",
        only: None,
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
        optional: &[],
        key: "id",
        only: None,
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
        optional: &[],
        key: "id",
        only: None,
    },
    // Kendi başına durur (başka tabloya başvurmaz); sunucuda tablo yoksa (0008 öncesi)
    // eşitlemenin geri kalanı sürer.
    Table {
        name: "settings",
        cols: &[
            ("id", Col::Text),
            ("value", Col::Text),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
        optional: &[],
        key: "key",
        only: Some(SYNCED_SETTINGS),
    },
    // Zaman çizelgesi satırları: kaydedilen, gönderilen ve silinen satırlar diğer cihazda da
    // aynı olur; aktarılmış satır orada bir daha gönderilmez. Projeye yabancı anahtarla bağlı
    // değildir. Sunucuda tablo yoksa (0010 öncesi) eşitlemenin geri kalanı sürer.
    Table {
        name: "timesheet_entries",
        cols: &[
            ("id", Col::Text),
            ("date", Col::Text),
            ("start", Col::Text),
            ("hours", Col::Real),
            ("kind", Col::Text),
            ("details", Col::Text),
            ("party", Col::Text),
            ("project_id", Col::Text),
            ("division", Col::Text),
            ("actual_hours", Col::OptReal),
            ("coverage", Col::OptText),
            ("timesheet_id", Col::OptText),
            ("exported_at", Col::OptTime),
            ("dismissed_at", Col::OptTime),
            ("created_at", Col::Time),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
        optional: &[],
        key: "id",
        only: None,
    },
];

/// Sonradan eklenen tablolar: sunucuda yoksa yalnızca onlar eşitlenmez, gerisi sürer.
const LATE_TABLES: &[&str] = &["settings", "timesheet_entries"];

impl Table {
    /// Sunucudaki sütunun yereldeki adı.
    fn local(&self, name: &'static str) -> &'static str {
        if name == "id" { self.key } else { name }
    }

    /// Yalnızca eşitlenen satırları seçen ek koşul (`only` sabit listesinden).
    fn filter(&self) -> String {
        self.only.map_or_else(String::new, |ids| {
            let ids: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
            format!(" AND {} IN ({})", self.key, ids.join(", "))
        })
    }

    fn allows(&self, id: &str) -> bool {
        self.only.is_none_or(|ids| ids.contains(&id))
    }
}

const PUSH_BATCH: usize = 500;
/// Kısa sayfa "hepsi çekildi" sayılır; sunucunun satır sınırının (PostgREST `max_rows`,
/// Supabase'de varsayılan 1000) altında kalmalı, yoksa her sayfa kısa görünür ve imleç
/// çekilmemiş satırların ötesine atlar.
const PULL_BATCH: usize = 500;
/// Bir çalıştırmada en fazla bu kadar parti (sonsuz döngüye karşı).
const MAX_BATCHES: usize = 200;
/// Eşzamanlı işlemlerde sunucu saatinin geride kalan satırlarını kaçırmamak için
/// her çalıştırmada imleç bu kadar geriden başlar (yeniden uygulamak zararsız). Sunucu
/// zamanı satır yazılırken damgalanır, işlem bitince değil: sunucudaki deyim zaman aşımından
/// (8 sn) rahatça uzun olmalı.
const CURSOR_OVERLAP_SECS: i64 = 60;
/// Satırı sunucuda son yazan cihazın sütunu (supabase/migrations/0003).
const WRITER: &str = "writer";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SyncSummary {
    pub pushed: usize,
    pub pulled: usize,
    /// Bu sürümün kabul etmediği (geçersiz ya da kısıtı ihlal eden) uzak satırlar.
    pub skipped: usize,
    /// Sunucu şeması eski (0007 çalıştırılmamış): proje arşivi ve bütçeler gönderilemedi;
    /// bunları taşıyan satırlar şema güncellenince yeniden gönderilir.
    pub outdated_schema: bool,
    /// Sunucuda ayarlar tablosu yok (0008 çalıştırılmamış): ayarlar eşitlenmedi.
    pub settings_unavailable: bool,
    /// Sunucuda zaman çizelgesi tablosu yok (0010 çalıştırılmamış): satırlar eşitlenmedi;
    /// tablo eklenince bekleyenler gönderilir.
    pub timesheet_unavailable: bool,
    /// Başka cihazdan ayar geldi: uygulama çalışan durumunu (gizlilik, hedefler, görünüm)
    /// yeniden okumalı.
    pub settings_pulled: bool,
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

impl SyncSummary {
    fn mark_unavailable(&mut self, table: &Table) {
        match table.name {
            "settings" => self.settings_unavailable = true,
            _ => self.timesheet_unavailable = true,
        }
    }
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
        match push_table(
            store,
            remote,
            table,
            user_id,
            &mut writer,
            &mut summary.outdated_schema,
        ) {
            Ok(n) => summary.pushed += n,
            Err(e) if missing_table(table, &e) => summary.mark_unavailable(table),
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
                summary.settings_pulled |= table.name == "settings" && n > 0;
            }
            Err(e) if missing_table(table, &e) => summary.mark_unavailable(table),
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

/// Sunucuda sonradan eklenen bir tablo ([`LATE_TABLES`]) yok mu? Bu durumda yalnızca o tablo
/// eşitlenmez; diğer tablolar hata vermeden sürer.
fn missing_table(table: &Table, e: &SyncError) -> bool {
    let SyncError::Remote(e) = e else {
        return false;
    };
    LATE_TABLES.contains(&table.name)
        && e.contains(table.name)
        && (e.contains("Could not find the table") || e.contains("does not exist"))
}

/// Sunucu şeması 0003'ten eskiyse `writer` sütunu bulunamaz.
fn missing_writer(e: &str) -> bool {
    e.contains(WRITER)
}

/// Sunucuda tablonun isteğe bağlı sütunlarından biri yok mu? PostgREST bilinmeyen sütunu
/// "Could not find the 'archived_at' column of 'tags' in the schema cache" diye reddeder.
/// Yalnızca bu kalıba bakılır: aynı sütunun bir kısıt hatası eksik sütun sayılmamalı.
fn missing_optional(table: &Table, e: &str) -> bool {
    table
        .optional
        .iter()
        .any(|c| e.contains(&format!("'{c}' column")) || e.contains(&format!("column \"{c}\"")))
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
    outdated: &mut bool,
) -> Result<usize, SyncError> {
    let mut total = 0;
    // Bu çalıştırmada isteğe bağlı sütunlar sunucuda yok mu?
    let mut full = true;
    for _ in 0..MAX_BATCHES {
        let rows = unsynced(&lock(store), table, PUSH_BATCH)?;
        if rows.is_empty() {
            break;
        }
        let payload = |writer: Option<&str>, full: bool| -> Vec<Value> {
            rows.iter()
                .map(|(row, _, _)| {
                    let mut row = row.clone();
                    if !full {
                        for c in table.optional {
                            row.remove(*c);
                        }
                    }
                    row.insert("user_id".into(), Value::String(user_id.into()));
                    if let Some(w) = writer {
                        row.insert(WRITER.into(), Value::String(w.into()));
                    }
                    Value::Object(row)
                })
                .collect()
        };
        let mut result = remote.push(table.name, &payload(*writer, full));
        // Eski şemada `writer` ve 0007 sütunları ayrı ayrı reddedilebilir: her biri bir kez.
        for _ in 0..2 {
            match &result {
                Err(e) if writer.is_some() && missing_writer(e) => *writer = None,
                Err(e) if full && missing_optional(table, e) => {
                    full = false;
                    *outdated = true;
                }
                _ => break,
            }
            result = remote.push(table.name, &payload(*writer, full));
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
        // Gönderilen sürüm işaretlenir; arada yerelde güncellenen satır kirli kalır. Eski
        // şemaya isteğe bağlı sütunları dolu olan satır eksik gitti: işaretlenmez, sunucu
        // güncellenince tamamı gönderilir (o satırlar her çalıştırmada yeniden gönderilir).
        let (complete, partial): (Vec<Pending>, Vec<Pending>) =
            rows.iter().cloned().partition(|(row, _, _)| {
                full || table
                    .optional
                    .iter()
                    .all(|c| row.get(*c).is_none_or(Value::is_null))
            });
        let marked = mark_synced(&lock(store), table, &complete)?;
        // Diğer cihazlar eksik sürümü aynı `updated_at` ile aldı; tam sürüm onlara ancak daha
        // yeniyse uygulanır. Bu yüzden bir milisaniye ilerletilir.
        bump(&lock(store), table, &partial)?;
        total += rows.len();
        if rows.len() < PUSH_BATCH || marked == 0 || !partial.is_empty() {
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
    let names: Vec<&str> = table.cols.iter().map(|c| table.local(c.0)).collect();
    let sql = format!(
        "SELECT {} FROM {} WHERE (synced_at IS NULL OR synced_at < updated_at){}
         ORDER BY updated_at LIMIT ?1",
        names.join(", "),
        table.name,
        table.filter()
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
                Col::Real => {
                    serde_json::Number::from_f64(r.get(i)?).map_or(Value::Null, Value::Number)
                }
                Col::Time => Value::String(iso(r.get(i)?)),
                Col::OptTime => r
                    .get::<_, Option<i64>>(i)?
                    .map_or(Value::Null, |v| Value::String(iso(v))),
                Col::OptReal => r
                    .get::<_, Option<f64>>(i)?
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number),
            };
            if *name == "id" {
                id = r.get(i)?;
            }
            if *name == "updated_at" {
                updated = r.get(i)?;
            }
            obj.insert(name.to_string(), value);
        }
        if table.name == "settings" {
            strip_local_fields(&mut obj);
        }
        Ok((obj, id, updated))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn mark_synced(store: &Store, table: &Table, rows: &[Pending]) -> Result<usize, SyncError> {
    let sql = format!(
        "UPDATE {} SET synced_at = ?2 WHERE {} = ?1 AND updated_at = ?2",
        table.name, table.key
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

/// Satırların `updated_at`'ini (arada değişmediyse) bir milisaniye ilerletir.
fn bump(store: &Store, table: &Table, rows: &[Pending]) -> Result<(), SyncError> {
    let sql = format!(
        "UPDATE {} SET updated_at = updated_at + 1 WHERE {} = ?1 AND updated_at = ?2",
        table.name, table.key
    );
    for (_, id, updated) in rows {
        store.conn().execute(&sql, rusqlite::params![id, updated])?;
    }
    Ok(())
}

/// Uzak satırı yerelde uygular; yerel sürüm daha yeniyse dokunmaz. Değişen satır sayısı.
fn apply_remote(store: &Store, table: &Table, row: &Value) -> Result<usize, SyncError> {
    let mut obj = row
        .as_object()
        .ok_or_else(|| SyncError::Invalid(row.to_string()))?
        .clone();
    let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
    if !table.allows(id) {
        return Err(SyncError::Invalid(format!("{}.{id}", table.name)));
    }
    if table.name == "settings" {
        let id = id.to_string();
        keep_local_fields(store, &id, &mut obj)?;
    }
    // Eski sunucunun hiç göndermediği isteğe bağlı sütunlar yazılmaz: yerel değer korunur.
    let cols: Vec<&(&str, Col)> = table
        .cols
        .iter()
        .filter(|(name, _)| !table.optional.contains(name) || obj.contains_key(*name))
        .collect();
    let mut values = Vec::with_capacity(cols.len() + 1);
    let mut updated = 0;
    for (name, col) in cols.iter().copied() {
        let v = obj.get(*name).unwrap_or(&Value::Null);
        let bad = || SyncError::Invalid(format!("{}.{name}: {v}", table.name));
        let sql = match (col, v) {
            (Col::Text, Value::String(s)) => SqlValue::Text(s.clone()),
            (Col::OptText, Value::String(s)) => SqlValue::Text(s.clone()),
            (Col::OptText | Col::OptTime | Col::OptReal, Value::Null) => SqlValue::Null,
            (Col::OptReal, Value::Number(n)) => SqlValue::Real(n.as_f64().ok_or_else(bad)?),
            (Col::Int, Value::Number(n)) => SqlValue::Integer(n.as_i64().ok_or_else(bad)?),
            (Col::Real, Value::Number(n)) => SqlValue::Real(n.as_f64().ok_or_else(bad)?),
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

    let names: Vec<&str> = cols.iter().map(|c| table.local(c.0)).collect();
    let placeholders: Vec<String> = (1..=names.len() + 1).map(|i| format!("?{i}")).collect();
    let updates: Vec<String> = names
        .iter()
        .skip(1)
        .chain(std::iter::once(&"synced_at"))
        .map(|n| format!("{n} = excluded.{n}"))
        .collect();
    let sql = format!(
        "INSERT INTO {t} ({cols}, synced_at) VALUES ({ph})
         ON CONFLICT ({key}) DO UPDATE SET {up}
         WHERE excluded.updated_at > {t}.updated_at",
        t = table.name,
        key = table.key,
        cols = names.join(", "),
        ph = placeholders.join(", "),
        up = updates.join(", "),
    );
    Ok(store.conn().execute(&sql, params_from_iter(values))?)
}

/// Uzaktan gelen gizlilik ayarına bu cihazın kendi alanlarını (duraklatma) yazar.
fn keep_local_fields(
    store: &Store,
    key: &str,
    obj: &mut Map<String, Value>,
) -> Result<(), SyncError> {
    let fields = local_fields(key);
    if fields.is_empty() {
        return Ok(());
    }
    let Some(Value::String(raw)) = obj.get("value") else {
        return Ok(());
    };
    let Ok(Value::Object(mut remote)) = serde_json::from_str::<Value>(raw) else {
        return Ok(());
    };
    let local: Option<Value> = store.setting(key)?;
    for field in fields {
        match local.as_ref().and_then(|l| l.get(*field)) {
            Some(v) => remote.insert((*field).into(), v.clone()),
            None => remote.remove(*field),
        };
    }
    obj.insert(
        "value".into(),
        Value::String(Value::Object(remote).to_string()),
    );
    Ok(())
}

/// Gönderilecek ayardan yalnızca bu cihazda kalan alanları çıkarır ([`LOCAL_FIELDS`]).
fn strip_local_fields(obj: &mut Map<String, Value>) {
    let fields = local_fields(obj.get("id").and_then(Value::as_str).unwrap_or_default());
    if fields.is_empty() {
        return;
    }
    let Some(Value::String(raw)) = obj.get("value") else {
        return;
    };
    let Ok(Value::Object(mut value)) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    for field in fields {
        value.remove(*field);
    }
    obj.insert(
        "value".into(),
        Value::String(Value::Object(value).to_string()),
    );
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
        /// Sunucuda olmayan sütunlar (0007 öncesi şema).
        missing: Vec<&'static str>,
        /// Sunucuda ayarlar tablosu yok (0008 öncesi şema).
        no_settings: bool,
        /// Sunucuda zaman çizelgesi tablosu yok (0010 öncesi şema).
        no_timesheet: bool,
    }

    const NO_SETTINGS: &str = "Could not find the table 'public.settings' in the schema cache";

    impl FakeRemote {
        /// Sunucuda olmayan tablonun PostgREST hatası.
        fn missing_table(&self, table: &str) -> Result<(), String> {
            if self.no_settings && table == "settings" {
                return Err(NO_SETTINGS.into());
            }
            if self.no_timesheet && table == "timesheet_entries" {
                return Err(
                    "Could not find the table 'public.timesheet_entries' in the schema cache"
                        .into(),
                );
            }
            Ok(())
        }
    }

    const NO_WRITER: &str = "Could not find the 'writer' column in the schema cache";

    impl Remote for FakeRemote {
        fn push(&mut self, table: &str, rows: &[Value]) -> Result<(), String> {
            self.pushes += 1;
            self.missing_table(table)?;
            if self.legacy && rows.iter().any(|r| r.get(WRITER).is_some()) {
                return Err(NO_WRITER.into());
            }
            if let Some(c) = self
                .missing
                .iter()
                .find(|c| rows.iter().any(|r| r.get(**c).is_some()))
            {
                return Err(format!(
                    "Could not find the '{c}' column of '{table}' in the schema cache"
                ));
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
            self.missing_table(table)?;
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
            self.missing_table(table)?;
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
    #[test]
    fn archive_and_budgets_sync_between_devices() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let project = lock(&a).accept_project_suggestion("Trumore").unwrap();
        let togg = crate::classify::Client {
            id: Uuid::new_v4().to_string(),
            name: "Togg".into(),
        };
        lock(&a).upsert_client(&togg, 0).unwrap();
        lock(&a)
            .set_project_budget(&project.id, Some(12.5))
            .unwrap();
        lock(&a).set_client_budget(&togg.id, Some(40.0)).unwrap();
        lock(&a).archive_project(&project.id).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();

        let extras = lock(&b).tag_extras().unwrap();
        let got = &extras[&project.id];
        assert_eq!(got.budget_days, Some(12.5));
        assert_eq!(
            got.archived_at.map(|t| t.timestamp_millis()),
            lock(&a).tag_extras().unwrap()[&project.id]
                .archived_at
                .map(|t| t.timestamp_millis())
        );
        assert_eq!(
            lock(&b).client_budgets().unwrap().get(&togg.id),
            Some(&40.0)
        );
        // Arşivdeki projenin kuralı B'de de sınıflandırmaz.
        assert!(
            lock(&b)
                .rules()
                .unwrap()
                .iter()
                .all(|r| r.tag_id != project.id)
        );

        // B arşivden çıkarıp bütçeyi kaldırır; A'ya ulaşır.
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&b).unarchive_project(&project.id).unwrap();
        lock(&b).set_project_budget(&project.id, None).unwrap();
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
        assert!(!lock(&a).tag_extras().unwrap().contains_key(&project.id));
        assert!(
            lock(&a)
                .rules()
                .unwrap()
                .iter()
                .any(|r| r.tag_id == project.id)
        );
    }

    #[test]
    fn old_server_without_0007_still_syncs_and_catches_up() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote {
            missing: vec!["archived_at", "budget_days"],
            ..Default::default()
        };
        let project = lock(&a).accept_project_suggestion("Trumore").unwrap();
        let plain = lock(&a).accept_project_suggestion("Kum").unwrap();
        lock(&a)
            .set_project_budget(&project.id, Some(10.0))
            .unwrap();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();

        // Eski şema: eşitleme sürer, yalnızca arşiv/bütçe gönderilemez ve bildirilir.
        let summary = run(&a, &mut remote, "u1").unwrap();
        assert!(summary.outdated_schema);
        assert!(remote.rows["tags"].contains_key(&plain.id));
        assert!(
            remote.rows["tags"]
                .values()
                .all(|r| r.get("budget_days").is_none())
        );
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(totals(&b), vec![("Code".to_string(), 60)]);
        assert_eq!(
            lock(&b).tags().unwrap().len(),
            lock(&a).tags().unwrap().len()
        );
        // B'nin yerel bütçesi, sütunu taşımayan uzak satırla silinmez.
        lock(&b).set_project_budget(&plain.id, Some(3.0)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&a)
            .upsert_tag(
                &Tag {
                    name: "Kum 2".into(),
                    ..plain.clone()
                },
                0,
            )
            .unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        let tags = lock(&b).tags().unwrap();
        assert_eq!(
            tags.iter().find(|t| t.id == plain.id).unwrap().name,
            "Kum 2"
        );
        assert_eq!(
            lock(&b).tag_extras().unwrap()[&plain.id].budget_days,
            Some(3.0)
        );

        // Sunucu güncellenince bekleyen bütçe kendiliğinden gönderilir.
        remote.missing.clear();
        let summary = run(&a, &mut remote, "u1").unwrap();
        assert!(!summary.outdated_schema);
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(
            lock(&b).tag_extras().unwrap()[&project.id].budget_days,
            Some(10.0)
        );
        assert_eq!(run(&a, &mut remote, "u1").unwrap().pushed, 0);
    }

    #[test]
    fn device_independent_settings_reach_a_new_device() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        {
            let a = lock(&a);
            a.save_setting("calendar_url", &Some("https://cal.example/ics"))
                .unwrap();
            a.save_setting("timesheet", &serde_json::json!({ "sheetToken": "t" }))
                .unwrap();
            a.save_setting("sync_auth", &"jeton").unwrap();
            a.save_setting("onboarded", &true).unwrap();
        }
        run(&a, &mut remote, "u1").unwrap();
        let ids: Vec<&String> = remote.rows["settings"].keys().collect();
        assert!(ids.iter().all(|k| SYNCED_SETTINGS.contains(&k.as_str())));

        let summary = run(&b, &mut remote, "u1").unwrap();
        assert!(summary.settings_pulled);
        let b = lock(&b);
        assert_eq!(
            b.setting::<Option<String>>("calendar_url").unwrap(),
            Some(Some("https://cal.example/ics".into()))
        );
        assert_eq!(
            b.setting::<Value>("timesheet").unwrap().unwrap()["sheetToken"],
            "t"
        );
        // Cihaza özgü ayarlar gelmez.
        assert_eq!(b.setting::<String>("sync_auth").unwrap(), None);
        assert_eq!(b.setting::<bool>("onboarded").unwrap(), None);
    }

    #[test]
    fn device_only_settings_from_remote_are_not_applied() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        remote
            .push(
                "settings",
                &[serde_json::json!({
                    "id": "sync_auth",
                    "value": "\"başkasının jetonu\"",
                    "updated_at": iso(1_800_000_000_000),
                    "deleted_at": null,
                })],
            )
            .unwrap();
        let summary = run(&a, &mut remote, "u1").unwrap();
        assert_eq!(summary.skipped, 1);
        assert_eq!(lock(&a).setting::<String>("sync_auth").unwrap(), None);
    }

    #[test]
    fn pausing_stays_on_its_own_device() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let mut privacy = crate::PrivacySettings {
            paused: true,
            ..Default::default()
        };
        privacy.excluded_apps = vec!["com.secret".into()];
        lock(&a).save_privacy_settings(&privacy).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();

        let got = lock(&b).privacy_settings().unwrap();
        assert_eq!(got.excluded_apps, vec!["com.secret".to_string()]);
        assert!(!got.paused);
    }

    #[test]
    fn the_ai_key_never_leaves_its_device() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        lock(&a)
            .save_setting(
                "ai_details",
                &serde_json::json!({ "enabled": true, "apiKey": "sk-ant-gizli" }),
            )
            .unwrap();
        lock(&b)
            .save_setting(
                "ai_details",
                &serde_json::json!({ "enabled": false, "apiKey": "sk-ant-b" }),
            )
            .unwrap();
        lock(&b).reset_sync_state().unwrap();
        run(&a, &mut remote, "u1").unwrap();
        let sent = remote.rows["settings"]["ai_details"].to_string();
        assert!(!sent.contains("sk-ant"), "{sent}");

        run(&b, &mut remote, "u1").unwrap();
        let got = lock(&b).setting::<Value>("ai_details").unwrap().unwrap();
        assert_eq!(got["enabled"], true);
        assert_eq!(got["apiKey"], "sk-ant-b");
    }

    #[test]
    fn signing_in_on_a_new_device_takes_the_accounts_settings() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        lock(&a).save_setting("theme", &"dark").unwrap();
        run(&a, &mut remote, "u1").unwrap();

        // Yeni cihaz girişten önce kendi (daha yeni) değerini kaydetmişti.
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&b).save_setting("theme", &"light").unwrap();
        lock(&b)
            .save_setting("goals", &serde_json::json!({ "dailyHours": 6 }))
            .unwrap();
        lock(&b).reset_sync_state().unwrap();
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();

        assert_eq!(
            lock(&b).setting::<String>("theme").unwrap().unwrap(),
            "dark"
        );
        assert_eq!(
            lock(&a).setting::<String>("theme").unwrap().unwrap(),
            "dark"
        );
        // Hesapta olmayan ayar yine de diğer cihaza geçer.
        assert_eq!(
            lock(&a).setting::<Value>("goals").unwrap().unwrap()["dailyHours"],
            6
        );
    }

    fn timesheet_row(details: &str) -> crate::timesheet::TimesheetEntry {
        crate::timesheet::TimesheetEntry {
            date: chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            hours: 1.25,
            actual_hours: Some(1.2),
            kind: crate::timesheet::EntryKind::Working,
            details: details.into(),
            party: "Ekip".into(),
            project_id: "p1".into(),
            division: "Yazılım".into(),
            coverage: Some(vec![[1_772_434_800_000, 1_772_439_120_000]]),
        }
    }

    #[test]
    fn timesheet_rows_follow_the_user_to_the_other_device() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();

        let kept = lock(&a)
            .save_timesheet_entry(None, &timesheet_row("Giriş ekranı"))
            .unwrap();
        let gone = lock(&a)
            .save_timesheet_entry(
                None,
                &crate::timesheet::TimesheetEntry {
                    coverage: Some(Vec::new()),
                    ..timesheet_row("Elle")
                },
            )
            .unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        let got = lock(&b).timesheet_entries(day, day).unwrap();
        assert_eq!(got.len(), 2);
        let row = got.iter().find(|s| s.id == kept).unwrap();
        assert_eq!(row.entry, timesheet_row("Giriş ekranı"));

        // A gönderir ve bir satırı siler; B'de satır gönderilmiş görünür, silinen kaybolur.
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&a)
            .mark_timesheet_exported(std::slice::from_ref(&kept), Utc::now(), "sheet")
            .unwrap();
        lock(&a).delete_timesheet_entry(&gone).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        let got = lock(&b).timesheet_entries(day, day).unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].exported_at.is_some());
        assert_eq!(got[0].timesheet_id.as_deref(), Some("sheet"));

        // B'de açıklama düzenlenir; aktarılmış satır değiştirilemediği için önce geri alınır.
        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&b)
            .unmark_timesheet_exported(std::slice::from_ref(&kept))
            .unwrap();
        lock(&b)
            .save_timesheet_entry(Some(&kept), &timesheet_row("Giriş ekranı testleri"))
            .unwrap();
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
        let got = lock(&a).timesheet_entries(day, day).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].entry.details, "Giriş ekranı testleri");
        assert!(got[0].exported_at.is_none());

        // Değişiklik yoksa bir şey gönderilmez.
        assert_eq!(run(&a, &mut remote, "u1").unwrap().pushed, 0);
    }

    #[test]
    fn undoing_a_merge_brings_the_rows_back_on_the_other_device() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let b = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote::default();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let first = timesheet_row("Bir");
        let second = crate::timesheet::TimesheetEntry {
            start: chrono::NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            coverage: Some(vec![[1_772_442_000_000, 1_772_445_600_000]]),
            ..timesheet_row("İki")
        };
        let ids: Vec<String> = [&first, &second]
            .iter()
            .map(|e| lock(&a).save_timesheet_entry(None, e).unwrap())
            .collect();
        let rows: Vec<_> = ids
            .iter()
            .zip([&first, &second])
            .map(|(id, e)| (Some(id.clone()), e.clone()))
            .collect();
        let (merged, removed) = lock(&a).merge_timesheet_entries(&rows).unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        assert_eq!(lock(&b).timesheet_entries(day, day).unwrap().len(), 1);

        std::thread::sleep(std::time::Duration::from_millis(5));
        lock(&a)
            .unmerge_timesheet_entries(&merged, &removed)
            .unwrap();
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
        let mut got: Vec<String> = lock(&b)
            .timesheet_entries(day, day)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        got.sort();
        let mut want = ids.clone();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn server_without_timesheet_table_still_syncs_the_rest() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote {
            no_timesheet: true,
            ..Default::default()
        };
        lock(&a)
            .save_timesheet_entry(None, &timesheet_row("Bekler"))
            .unwrap();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        let summary = run(&a, &mut remote, "u1").unwrap();
        assert!(summary.timesheet_unavailable);
        assert_eq!(remote.rows["sessions"].len(), 1);

        remote.no_timesheet = false;
        run(&a, &mut remote, "u1").unwrap();
        assert_eq!(remote.rows["timesheet_entries"].len(), 1);
    }

    #[test]
    fn server_without_settings_table_still_syncs_the_rest() {
        let a = Mutex::new(Store::open_in_memory().unwrap());
        let mut remote = FakeRemote {
            no_settings: true,
            ..Default::default()
        };
        lock(&a).save_setting("theme", &"dark").unwrap();
        lock(&a).upsert_session(&session("Code", 60)).unwrap();
        let summary = run(&a, &mut remote, "u1").unwrap();
        assert!(summary.settings_unavailable);
        assert_eq!(remote.rows["sessions"].len(), 1);

        // Tablo eklenince bekleyen ayar gönderilir.
        remote.no_settings = false;
        run(&a, &mut remote, "u1").unwrap();
        assert!(remote.rows["settings"].contains_key("theme"));
    }
}
