//! Sunucu şeması denetimi: `supabase/migrations/` altındaki göçlerden hangileri Supabase
//! projesinde çalıştırılmamış? Her göçün eklediği bir tablo ya da sütun yoklanır (boş bir
//! `select`); bulunamayan göç eksik sayılır. Kısıt ya da yetki değiştiren göçler (0006, 0009)
//! yoklanamaz; bunlar ayrıca listelenir.
//!
//! Eşitleme eksik göçleri zaten tolere eder (yalnızca ilgili veri taşınmaz); bu denetim
//! kullanıcıya hangi dosyaları çalıştırması gerektiğini tek yerde söyler.

/// Bir göç dosyası ve onun sunucuda bıraktığı iz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// `supabase/migrations/` altındaki dosya adı.
    pub file: &'static str,
    /// Göçün eklediği tablo ve sütun; `None`: yoklanamaz (kısıt, yetki).
    pub probe: Option<(&'static str, &'static str)>,
    /// Göç çalıştırılmazsa ne olur (kullanıcıya gösterilir).
    pub effect: &'static str,
    /// Dosyanın içeriği (`public` şeması için).
    pub sql: &'static str,
}

macro_rules! migration {
    ($file:literal, $probe:expr, $effect:literal) => {
        Migration {
            file: $file,
            probe: $probe,
            effect: $effect,
            sql: include_str!(concat!("../../../../supabase/migrations/", $file)),
        }
    };
}

/// Bütün göçler, sırayla. Yeni göç eklenince buraya da eklenir (test denetler).
pub const MIGRATIONS: &[Migration] = &[
    migration!(
        "0001_kum_sync.sql",
        Some(("sessions", "id")),
        "Temel tablolar yok: hiçbir şey eşitlenmez."
    ),
    migration!(
        "0002_session_category.sql",
        Some(("sessions", "category_id")),
        "Oturumlar eşitlenemez."
    ),
    migration!(
        "0003_writer.sql",
        Some(("sessions", "writer")),
        "Her bilgisayar kendi gönderdiklerini de geri indirir (eşitleme yavaşlar)."
    ),
    migration!(
        "0004_session_project.sql",
        Some(("sessions", "project_id")),
        "Oturumlar eşitlenemez."
    ),
    migration!(
        "0005_clients.sql",
        Some(("clients", "id")),
        "Müşteriler eşitlenemez."
    ),
    migration!(
        "0006_domain_rules.sql",
        None,
        "Web sitesi kuralı eklenince eşitleme \"rules_field_check\" hatası verir."
    ),
    migration!(
        "0007_archive_budget.sql",
        Some(("tags", "archived_at")),
        "Proje arşivi ve sözleşme bütçeleri öteki bilgisayarlara geçmez."
    ),
    migration!(
        "0008_settings.sql",
        Some(("settings", "id")),
        "Ayarlar (zaman çizelgeleri, takvim, gizlilik, hedefler) taşınmaz."
    ),
    migration!(
        "0009_harden_grants.sql",
        None,
        "Gerekli değil; tablo yetkilerini daraltır (önerilir)."
    ),
    migration!(
        "0010_timesheet_entries.sql",
        Some(("timesheet_entries", "id")),
        "Zaman çizelgesi satırları eşitlenmez."
    ),
    migration!(
        "0011_timesheet_state.sql",
        Some(("timesheet_entries", "consultant")),
        "Zaman çizelgesi satırlarının durumu ve danışmanı eşitlenmez."
    ),
    migration!(
        "0012_session_state.sql",
        Some(("sessions", "state_at")),
        "Başka bilgisayarda süren oturum, bu arada yapılan atamayı geri alabilir."
    ),
    migration!(
        "0013_session_block_from.sql",
        Some(("sessions", "block_from")),
        "Takvimde elle bölünen bloklar öteki bilgisayarda bölünmez."
    ),
    migration!(
        "0014_calls.sql",
        Some(("calls", "id")),
        "Görüşmeler eşitlenmez; toplantı katılımı yalnızca bu bilgisayarla yargılanır."
    ),
    migration!(
        "0015_session_passive.sql",
        Some(("sessions", "passive")),
        "Girdisiz izleme (video, görüşme) öteki bilgisayara geçmez."
    ),
    migration!(
        "0016_timesheet_marks.sql",
        Some(("timesheet_marks", "id")),
        "Zaman çizelgesine gönderilen bloklar öteki bilgisayarda görünmez."
    ),
    migration!(
        "0017_sync_nudges.sql",
        Some(("sync_nudges", "writer")),
        "Anında eşitleme kapalı: değişiklikler öteki bilgisayara en geç 5 dakikada gider."
    ),
    migration!(
        "0018_plan_blocks.sql",
        Some(("plan_blocks", "id")),
        "Gün planı eşitlenmez: her bilgisayar kendi planını görür."
    ),
];

/// Ayrı şemada (`kum`) kurulum: bütün göçlerin tek dosyalık hâli (tekrar çalıştırmak zararsız).
pub const KUM_SCHEMA_SQL: &str = include_str!("../../../../supabase/kum_schema.sql");

/// Denetimin sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCheck {
    /// Sunucuda izi bulunmayan göçler, sırayla.
    pub missing: Vec<&'static Migration>,
    /// Yoklanamayan göçler (çalıştırılıp çalıştırılmadığı bilinemez).
    pub unchecked: Vec<&'static Migration>,
}

impl SchemaCheck {
    /// Çalıştırılacak SQL. `public` şemasında eksik göçler sırayla (yoklanamayanlar dahil
    /// değil); ayrı şemada `kum_schema.sql` bütünüyle. Eksik yoksa boş.
    pub fn sql(&self, custom_schema: bool) -> String {
        if self.missing.is_empty() {
            return String::new();
        }
        if custom_schema {
            return KUM_SCHEMA_SQL.to_string();
        }
        self.missing
            .iter()
            .map(|m| format!("-- ===== {} =====\n{}", m.file, m.sql.trim_end()))
            .collect::<Vec<_>>()
            .join("\n\n")
            + "\n"
    }
}

/// Göçleri yoklar. `probe(tablo, sütun)`: sunucuda varsa `Ok(true)`, yoksa `Ok(false)`;
/// başka bir hata (ağ, oturum) denetimi durdurur. Bir tablo yoksa aynı tablonun sonraki
/// sütunları yoklanmadan eksik sayılır.
pub fn check(
    probe: &mut dyn FnMut(&str, &str) -> Result<bool, String>,
) -> Result<SchemaCheck, String> {
    let mut missing = Vec::new();
    let mut unchecked = Vec::new();
    let mut absent_tables: Vec<&str> = Vec::new();
    for m in MIGRATIONS {
        let Some((table, column)) = m.probe else {
            unchecked.push(m);
            continue;
        };
        let present = !absent_tables.contains(&table) && probe(table, column)?;
        if !present {
            // Tabloyu bu göç kuruyorsa tablo yok: sonraki sütunları yoklamaya gerek kalmaz.
            if m.sql
                .contains(&format!("create table if not exists public.{table} "))
            {
                absent_tables.push(table);
            }
            missing.push(m);
        }
    }
    Ok(SchemaCheck { missing, unchecked })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&Migration]) -> Vec<&'static str> {
        list.iter().map(|m| m.file).collect()
    }

    #[test]
    fn every_migration_file_is_listed_in_order() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../supabase/migrations");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        files.sort();
        let listed: Vec<&str> = MIGRATIONS.iter().map(|m| m.file).collect();
        assert_eq!(files, listed);
    }

    #[test]
    fn probes_name_what_the_migration_creates() {
        for m in MIGRATIONS {
            if let Some((table, column)) = m.probe {
                let sql = m.sql.to_lowercase();
                assert!(sql.contains(&format!("public.{table}")), "{}", m.file);
                assert!(sql.contains(column), "{}: {column}", m.file);
            }
        }
    }

    #[test]
    fn up_to_date_server_has_nothing_missing() {
        let got = check(&mut |_, _| Ok(true)).unwrap();
        assert!(got.missing.is_empty());
        assert_eq!(
            names(&got.unchecked),
            ["0006_domain_rules.sql", "0009_harden_grants.sql"]
        );
        assert_eq!(got.sql(false), "");
    }

    #[test]
    fn missing_tables_and_columns_are_reported_with_their_sql() {
        let mut asked = Vec::new();
        let got = check(&mut |table, column| {
            asked.push(format!("{table}.{column}"));
            Ok(!matches!(table, "plan_blocks" | "sync_nudges") && column != "passive")
        })
        .unwrap();
        assert_eq!(
            names(&got.missing),
            [
                "0015_session_passive.sql",
                "0017_sync_nudges.sql",
                "0018_plan_blocks.sql"
            ]
        );
        let sql = got.sql(false);
        assert!(sql.starts_with("-- ===== 0015_session_passive.sql ====="));
        assert!(sql.contains("create table if not exists public.plan_blocks"));
        assert!(!sql.contains("0006"));
        // Ayrı şemada tek dosya.
        assert!(got.sql(true).contains("create schema if not exists kum"));
        assert!(asked.contains(&"sessions.passive".to_string()));
    }

    #[test]
    fn a_missing_table_is_not_probed_again() {
        let mut asked = Vec::new();
        let got = check(&mut |table, column| {
            asked.push(format!("{table}.{column}"));
            Ok(table != "timesheet_entries")
        })
        .unwrap();
        assert_eq!(
            names(&got.missing),
            ["0010_timesheet_entries.sql", "0011_timesheet_state.sql"]
        );
        assert!(!asked.contains(&"timesheet_entries.consultant".to_string()));
    }

    #[test]
    fn other_errors_stop_the_check() {
        let err = check(&mut |_, _| Err("bağlanılamadı".into())).unwrap_err();
        assert_eq!(err, "bağlanılamadı");
    }
}
