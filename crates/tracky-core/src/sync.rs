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
use rusqlite::{OptionalExtension, params_from_iter, types::Value as SqlValue};
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
    /// `only`'ye ek olarak bu önekle başlayan kimlikler de eşitlenir.
    prefix: Option<&'static str>,
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
    "meeting_attendance",
    "closed_days",
];

/// Parça parça şekle ([`table_fingerprints`]) geçilmeden önce eklenmiş tablo ve ayarlar:
/// [`legacy_fingerprint`] bunlarsız hesaplanır.
const AFTER_LEGACY_TABLES: &[&str] = &["calls"];
const AFTER_LEGACY_SETTINGS: &[&str] = &["meeting_attendance", "closed_days"];

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
        prefix: None,
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
        prefix: None,
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
        prefix: None,
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
            ("state_at", Col::OptTime),
            ("block_from", Col::OptTime),
        ],
        // 0012: atama ve silinmenin zamanı; 0013: takvim bloğunun elle bölündüğü an.
        optional: &["state_at", "block_from"],
        key: "id",
        only: None,
        prefix: None,
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
        // Bilgisayar adları (`device:<kimlik>`); her bilgisayar kendi satırını yazar.
        prefix: Some(crate::store::DEVICE_KEY_PREFIX),
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
            ("state_at", Col::OptTime),
            ("consultant", Col::OptText),
        ],
        // 0011: durum zamanı ve danışman adı.
        optional: &["state_at", "consultant"],
        key: "id",
        only: None,
        prefix: None,
    },
    // Görüşmeler (crate::calls): toplantı katılımı başka bilgisayarda da aynı yargılanır.
    // Sunucuda tablo yoksa (0014 öncesi) eşitlemenin geri kalanı sürer.
    Table {
        name: "calls",
        cols: &[
            ("id", Col::Text),
            ("device_id", Col::Text),
            ("app_id", Col::Text),
            ("started_at", Col::Time),
            ("ended_at", Col::Time),
            ("updated_at", Col::Time),
            ("deleted_at", Col::OptTime),
        ],
        optional: &[],
        key: "id",
        only: None,
        prefix: None,
    },
];

/// Sonradan eklenen tablolar: sunucuda yoksa yalnızca onlar eşitlenmez, gerisi sürer.
const LATE_TABLES: &[&str] = &["settings", "timesheet_entries", "calls"];

impl Table {
    /// Sunucudaki sütunun yereldeki adı.
    fn local(&self, name: &'static str) -> &'static str {
        if name == "id" { self.key } else { name }
    }

    /// Yalnızca eşitlenen satırları seçen ek koşul (`only` sabit listesinden).
    fn filter(&self) -> String {
        self.only.map_or_else(String::new, |ids| {
            let ids: Vec<String> = ids.iter().map(|id| format!("'{id}'")).collect();
            let prefix = self
                .prefix
                .map_or_else(String::new, |p| format!(" OR {} LIKE '{p}%'", self.key));
            format!(" AND ({} IN ({}){prefix})", self.key, ids.join(", "))
        })
    }

    fn allows(&self, id: &str) -> bool {
        self.only.is_none_or(|ids| ids.contains(&id))
            || self.prefix.is_some_and(|p| id.starts_with(p))
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
    /// Sunucuda zaman çizelgesinin 0011 sütunları yok: satırların durumu ve danışmanı
    /// eşitlenmedi (satırlar bütün olarak eşitlenir); şema güncellenince yeniden gönderilir.
    pub timesheet_outdated: bool,
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

/// Tablonun eşitlenen şekli (sütunlar, isteğe bağlı sütunlar, eşitlenen kimlikler).
fn table_shape(t: &Table, only: Option<&[&str]>) -> String {
    let mut text = t.name.to_string();
    for (name, col) in t.cols {
        text.push_str(&format!(",{name}:{col:?}"));
    }
    text.push_str(&format!(";{:?};{only:?};{:?}|", t.optional, t.prefix));
    text
}

/// FNV-1a: derlemeler arasında kararlı (std hasher değil).
fn fnv(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// Her tablonun eşitlenen şeklinin kısa özeti. Eski sürümün tanımayıp atladığı satırları (yeni
/// sütun, yeni ayar) imleç geçtiği için bir daha çekmemek yerine yalnızca şekli değişen tablo
/// baştan çekilir; her sürümde ve her değişiklikte bütün tablolar değil.
pub fn table_fingerprints() -> Vec<(&'static str, String)> {
    TABLES
        .iter()
        .map(|t| {
            let mut text = table_shape(t, t.only);
            if t.name == "settings" {
                text.push_str(&format!("{LOCAL_FIELDS:?}"));
            }
            (t.name, fnv(&text))
        })
        .collect()
}

/// Tablolar tek bir özetle izlenirken (0.9.44'e kadar) kaydedilen özet. Bu özet kayıtlıysa
/// tablolar o şekilde eşitlenmiştir: parça parça özete geçerken baştan çekilmesi gerekmez
/// (yalnızca sonradan eklenenler).
pub fn legacy_fingerprint() -> String {
    let mut text = String::new();
    for t in TABLES
        .iter()
        .filter(|t| !AFTER_LEGACY_TABLES.contains(&t.name))
    {
        let only: Option<Vec<&str>> = t.only.map(|ids| {
            ids.iter()
                .copied()
                .filter(|id| !AFTER_LEGACY_SETTINGS.contains(id))
                .collect()
        });
        text.push_str(&table_shape(t, only.as_deref()));
    }
    text.push_str(&format!("{LOCAL_FIELDS:?}"));
    fnv(&text)
}

/// Şekli parça parça özete göre baştan çekilmesi gereken tablolar. `stored` tablonun kayıtlı
/// özeti, `legacy` eski tek özet kayıtlı ve [`legacy_fingerprint`] ile aynı mı.
pub fn tables_to_refetch(
    stored: impl Fn(&str) -> Option<String>,
    legacy: bool,
) -> Vec<(&'static str, String, bool)> {
    table_fingerprints()
        .into_iter()
        .filter_map(|(name, fp)| match stored(name) {
            Some(s) if s == fp => None,
            // Eski özetle eşitlenmiş tablo zaten bu şekildedir; yalnızca özeti kaydedilir.
            None if legacy && !AFTER_LEGACY_TABLES.contains(&name) && name != "settings" => {
                Some((name, fp, false))
            }
            _ => Some((name, fp, true)),
        })
        .collect()
}

impl SyncSummary {
    fn mark_unavailable(&mut self, table: &Table) {
        match table.name {
            "settings" => self.settings_unavailable = true,
            // Görüşmeler eşitlenmezse katılım yalnızca bu bilgisayarın görüşmeleriyle yargılanır.
            "calls" => {}
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
            if table.name == "timesheet_entries" {
                &mut summary.timesheet_outdated
            } else {
                &mut summary.outdated_schema
            },
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
            if table.name != "sessions" {
                SyncError::Remote(e)
            } else if e.contains("category_id") {
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
    let merge = match table.name {
        "timesheet_entries" => Some(&TIMESHEET_MERGE),
        "sessions" => Some(&SESSION_MERGE),
        _ => None,
    }
    .filter(|_| obj.contains_key("state_at"));
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
    let names: Vec<&str> = cols.iter().map(|c| table.local(c.0)).collect();
    if let Some(merge) = merge
        && let Some(n) = merge_row(store, table.name, merge, &names, &values)?
    {
        return Ok(n);
    }
    values.push(SqlValue::Integer(updated)); // synced_at

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

/// Durumu içerikten ayrı, `state_at`'e göre birleşen satırlar ([`merge_row`]).
struct Merge {
    /// Durum alanları (`state_at` dahil); geri kalan içeriktir.
    state: &'static [&'static str],
    /// Bunlardan biri doluysa satır donmuştur: durumu daha yeni taraf onu böyle bıraktıysa
    /// içerik de o taraftan gelir (öteki taraf bunu bilmeden düzenledi).
    frozen: &'static [&'static str],
    /// Durum zamanı boş satırda yerine geçen: zaman çizelgesinde `updated_at` (sütundan önceki
    /// satırlar); oturumda hiçbir şey (hiç düzenlenmemiş satır en eski sayılır, süren oturumu
    /// uzatan takip başka cihazdaki atamayı ezmesin).
    fallback_to_updated: bool,
}

/// Zaman çizelgesi satırının durumu: aktarım, gizlenme, silinme, danışman.
const TIMESHEET_MERGE: Merge = Merge {
    state: &[
        "timesheet_id",
        "exported_at",
        "dismissed_at",
        "deleted_at",
        "consultant",
        "state_at",
    ],
    frozen: &["exported_at", "deleted_at"],
    fallback_to_updated: true,
};

/// Oturumun durumu: elle atama ve silinme. İçerik (saatler, başlık) son yazandan gelir; süren
/// oturumu takip eden cihaz onu uzatmaya devam eder.
const SESSION_MERGE: Merge = Merge {
    state: &[
        "category_id",
        "project_id",
        "block_from",
        "deleted_at",
        "state_at",
    ],
    frozen: &[],
    fallback_to_updated: false,
};

/// Uzaktan gelen satırı yereldekiyle alan alan birleştirir; yerelde satır yoksa `None`
/// (olduğu gibi eklenir). Bütün satırda "son yazan kazanır" olsaydı, eşitlenmemiş eski kopyayı
/// düzenleyen cihaz aktarımı ya da silmeyi, süren oturumu uzatan cihaz da öteki cihazdaki
/// atamayı ya da silmeyi geri alırdı:
/// - durum daha yeni `state_at`'li taraftan gelir (eşitlikte daha yeni `updated_at`);
/// - içerik daha yeni `updated_at`'li taraftan; ama durumu daha yeni olan taraf satırı
///   dondurmuşsa ([`Merge::frozen`]) içerik de ondan.
///
/// Sonuç gelen satırdan farklıysa satır kirli kalır ve birleşmiş hali geri gönderilir; böylece
/// sunucu ve öteki cihaz da aynı sonuca varır.
fn merge_row(
    store: &Store,
    table: &str,
    merge: &Merge,
    names: &[&str],
    incoming: &[SqlValue],
) -> Result<Option<usize>, SyncError> {
    let at = |name: &str| names.iter().position(|n| *n == name).expect("tablo sütunu");
    let local: Option<Vec<SqlValue>> = store
        .conn()
        .query_row(
            &format!("SELECT {} FROM {table} WHERE id = ?1", names.join(", ")),
            [&incoming[at("id")]],
            |r| (0..names.len()).map(|i| r.get::<_, SqlValue>(i)).collect(),
        )
        .optional()?;
    let Some(local) = local else {
        return Ok(None);
    };
    let int = |row: &[SqlValue], name: &str| match row[at(name)] {
        SqlValue::Integer(v) => Some(v),
        _ => None,
    };
    let (iu, lu) = (
        int(incoming, "updated_at").unwrap_or(0),
        int(&local, "updated_at").unwrap_or(0),
    );
    let state_at = |row: &[SqlValue], updated| {
        int(row, "state_at").unwrap_or(if merge.fallback_to_updated {
            updated
        } else {
            0
        })
    };
    let (is, ls) = (state_at(incoming, iu), state_at(&local, lu));
    let state_in = (is, iu) > (ls, lu);
    let frozen = |row: &[SqlValue]| merge.frozen.iter().any(|c| row[at(c)] != SqlValue::Null);
    let content_in = match is.cmp(&ls) {
        std::cmp::Ordering::Greater if frozen(incoming) => true,
        std::cmp::Ordering::Less if frozen(&local) => false,
        _ => iu > lu,
    };
    let merged: Vec<SqlValue> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let from_incoming = match *name {
                "id" | "created_at" | "updated_at" => false,
                n if merge.state.contains(&n) => state_in,
                _ => content_in,
            };
            if from_incoming {
                incoming[i].clone()
            } else {
                local[i].clone()
            }
        })
        .collect();
    let same = |row: &[SqlValue]| {
        names
            .iter()
            .enumerate()
            .filter(|(_, n)| !matches!(**n, "created_at" | "updated_at"))
            .all(|(i, _)| merged[i] == row[i])
    };
    let (updated, synced) = if same(incoming) && iu >= lu {
        // Gelen satır olduğu gibi geçerli: eşitlenmiş sayılır.
        (iu, Some(iu))
    } else if same(&local) && iu <= lu {
        return Ok(Some(0));
    } else {
        // Birleşen hal iki taraftan da yeni: gönderilmek üzere kirli kalır.
        (iu.max(lu) + 1, None)
    };
    let sets: Vec<String> = names
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, n)| !matches!(**n, "created_at" | "updated_at"))
        .map(|(i, n)| format!("{n} = ?{}", i + 1))
        .collect();
    let mut values = merged;
    values.push(SqlValue::Integer(updated));
    values.push(synced.map_or(SqlValue::Null, SqlValue::Integer));
    let (u, s) = (values.len() - 1, values.len());
    store.conn().execute(
        &format!(
            "UPDATE {table} SET {}, updated_at = ?{u},
                synced_at = COALESCE(?{s}, synced_at)
             WHERE id = ?1",
            sets.join(", ")
        ),
        params_from_iter(values),
    )?;
    Ok(Some(1))
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
mod tests;
