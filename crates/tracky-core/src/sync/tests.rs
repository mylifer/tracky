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

#[test]
fn legacy_fingerprint_matches_the_single_summary_of_0_9_44() {
    // 0.9.44'ün kaydettiği özet: bu eşleşmezse güncellemede bütün tablolar baştan çekilir.
    assert_eq!(legacy_fingerprint(), "24e642a5975c0eb2");
}

#[test]
fn only_new_or_changed_tables_are_refetched() {
    let current: HashMap<&str, String> = table_fingerprints().into_iter().collect();
    assert_eq!(current.len(), TABLES.len());
    // Eski tek özetten geçiş: yalnızca yeni tablo ve yeni ayar eklenen ayarlar baştan çekilir.
    let first = tables_to_refetch(|_| None, true);
    let refetch: Vec<&str> = first.iter().filter(|r| r.2).map(|r| r.0).collect();
    assert_eq!(refetch, ["settings", "calls"]);
    assert_eq!(first.len(), TABLES.len());
    // Kayıtlıyla aynıysa hiçbir şey; biri değişmişse yalnızca o.
    let stored = |name: &str| current.get(name).cloned();
    assert!(tables_to_refetch(stored, false).is_empty());
    let changed = tables_to_refetch(
        |name: &str| (name != "tags").then(|| current[name].clone()),
        false,
    );
    assert_eq!(
        changed.iter().map(|r| (r.0, r.2)).collect::<Vec<_>>(),
        [("tags", true)]
    );
    // Eski özet de yoksa (çok eski sürüm ya da ilk eşitleme) hepsi.
    assert!(tables_to_refetch(|_| None, false).iter().all(|r| r.2));
}

impl FakeRemote {
    /// Sunucuda olmayan tablonun PostgREST hatası.
    fn missing_table(&self, table: &str) -> Result<(), String> {
        if self.no_settings && table == "settings" {
            return Err(NO_SETTINGS.into());
        }
        if self.no_timesheet && table == "timesheet_entries" {
            return Err(
                "Could not find the table 'public.timesheet_entries' in the schema cache".into(),
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
            since.is_none_or(|s| parse_time(r["server_updated_at"].as_str().unwrap()).unwrap() > s)
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
        block_from: None,
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

/// Süren oturumu uzatan cihaz, öteki cihazdaki proje atamasını ve silmeyi ezmez: eşitleme
/// gecikince (öteki cihaz oturumu bitmiş sanıp düzenler) satırın tamamı son yazanla
/// eşitlenseydi atama ya da silme sessizce geri alınırdı.
#[test]
fn extending_a_live_session_keeps_the_other_devices_assignment() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    let mut s = session("Figma", 600);
    lock(&a).upsert_session(&s).unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    let project = lock(&b).accept_project_suggestion("Trumore").unwrap();
    pause();
    lock(&b)
        .set_project_between(s.started_at, s.ended_at, Some(&project.id))
        .unwrap();
    // A'nın takibi aynı oturumu uzatmaya devam eder (B'nin atamasından sonra).
    pause();
    s.ended_at += Duration::seconds(60);
    lock(&a).upsert_session(&s).unwrap();
    // A'nın gönderdiği uzamış satır sunucudakini ezer; B birleştirip geri gönderir.
    for _ in 0..3 {
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
    }
    for store in [&a, &b] {
        let got = lock(store)
            .sessions_between(s.started_at, s.ended_at)
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(got[0].ended_at, s.ended_at);
    }
    assert_eq!(run(&a, &mut remote, "u1").unwrap().pushed, 0);
    assert_eq!(run(&b, &mut remote, "u1").unwrap().pushed, 0);
}

#[test]
fn a_session_deleted_on_another_device_stays_deleted_while_tracking_goes_on() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    let mut s = session("Figma", 600);
    lock(&a).upsert_session(&s).unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    pause();
    lock(&b).delete_between(s.started_at, s.ended_at).unwrap();
    pause();
    s.ended_at += Duration::seconds(60);
    lock(&a).upsert_session(&s).unwrap();
    for _ in 0..3 {
        run(&b, &mut remote, "u1").unwrap();
        run(&a, &mut remote, "u1").unwrap();
    }
    for store in [&a, &b] {
        assert!(
            lock(store)
                .sessions_between(s.started_at, s.ended_at)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn computer_names_reach_the_other_device() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    lock(&a)
        .register_device("Mac Studio", "macos", "Mac Studio")
        .unwrap();
    lock(&b).register_device("Ofis PC", "windows", "").unwrap();
    for _ in 0..2 {
        run(&a, &mut remote, "u1").unwrap();
        run(&b, &mut remote, "u1").unwrap();
    }
    let a_id = lock(&a).device_id().to_string();
    // B, A'yı yeniden adlandırır; ad A'ya da gider.
    lock(&b).rename_device(&a_id, "Ev Mac'i").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    for store in [&a, &b] {
        let mut names: Vec<String> = lock(store)
            .known_devices()
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["Ev Mac'i".to_string(), "Ofis PC".to_string()]);
    }
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
        .mark_timesheet_exported(std::slice::from_ref(&kept), Utc::now(), "sheet", "")
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

/// İki cihaz eşitlenir; `id`'li satırı ikisinde de döndürür.
fn in_both(a: &Mutex<Store>, b: &Mutex<Store>, remote: &mut FakeRemote, id: &str) {
    run(a, remote, "u1").unwrap();
    run(b, remote, "u1").unwrap();
    assert!(lock(a).timesheet_entry(id).unwrap().is_some());
    assert!(lock(b).timesheet_entry(id).unwrap().is_some());
}

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

#[test]
fn a_stale_edit_on_another_device_does_not_undo_an_export() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    let id = lock(&a)
        .save_timesheet_entry(None, &timesheet_row("Giriş ekranı"))
        .unwrap();
    in_both(&a, &b, &mut remote, &id);

    // A aktarır; eşitlenmemiş B eski kopyayı düzenler ve önce o eşitler.
    pause();
    lock(&a)
        .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), "sheet", "Kaan")
        .unwrap();
    pause();
    lock(&b)
        .save_timesheet_entry(Some(&id), &timesheet_row("Eski kopyada düzenlendi"))
        .unwrap();
    run(&b, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    // Satır iki cihazda da aktarılmış kalır ve dosyadaki haliyle durur (yeniden gönderilmez).
    for store in [&a, &b] {
        let row = lock(store).timesheet_entry(&id).unwrap().unwrap();
        assert!(row.exported_at.is_some());
        assert_eq!(row.timesheet_id.as_deref(), Some("sheet"));
        assert_eq!(row.consultant.as_deref(), Some("Kaan"));
        assert_eq!(row.entry.details, "Giriş ekranı");
    }
    assert_eq!(run(&a, &mut remote, "u1").unwrap().pushed, 0);
    assert_eq!(run(&b, &mut remote, "u1").unwrap().pushed, 0);
}

#[test]
fn a_stale_edit_does_not_bring_back_merged_rows() {
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
    in_both(&a, &b, &mut remote, &ids[0]);
    pause();
    let rows: Vec<_> = ids
        .iter()
        .zip([&first, &second])
        .map(|(id, e)| (Some(id.clone()), e.clone()))
        .collect();
    let (merged, _) = lock(&a).merge_timesheet_entries(&rows).unwrap();
    pause();
    lock(&b)
        .save_timesheet_entry(Some(&ids[0]), &timesheet_row("Bir, düzenlendi"))
        .unwrap();
    run(&b, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    for store in [&a, &b] {
        let live: Vec<String> = lock(store)
            .timesheet_entries(day, day)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(live, std::slice::from_ref(&merged));
    }
}

#[test]
fn the_same_suggestion_saved_on_two_devices_is_one_row() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
    let on_a = lock(&a)
        .save_timesheet_entry(None, &timesheet_row("A'da yazıldı"))
        .unwrap();
    pause();
    let on_b = lock(&b)
        .save_timesheet_entry(None, &timesheet_row("B'de yazıldı"))
        .unwrap();
    assert_eq!(on_a, on_b, "aynı işin satırı aynı kimliği alır");
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    for store in [&a, &b] {
        let rows = lock(store).timesheet_entries(day, day).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.details, "B'de yazıldı");
    }
}

#[test]
fn deleting_on_one_device_and_editing_on_the_other_keeps_both() {
    let a = Mutex::new(Store::open_in_memory().unwrap());
    let b = Mutex::new(Store::open_in_memory().unwrap());
    let mut remote = FakeRemote::default();
    let id = lock(&a)
        .save_timesheet_entry(None, &timesheet_row("Giriş ekranı"))
        .unwrap();
    in_both(&a, &b, &mut remote, &id);
    // A gizler, B (bilmeden) açıklamayı düzenler: satır gizli kalır, açıklama B'nin.
    pause();
    let row = timesheet_row("Giriş ekranı");
    lock(&a).dismiss_timesheet_entry(Some(&id), &row).unwrap();
    pause();
    lock(&b)
        .save_timesheet_entry(Some(&id), &timesheet_row("Giriş ekranı testleri"))
        .unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&a, &mut remote, "u1").unwrap();
    run(&b, &mut remote, "u1").unwrap();
    for store in [&a, &b] {
        let row = lock(store).timesheet_entry(&id).unwrap().unwrap();
        assert!(row.dismissed);
        assert_eq!(row.entry.details, "Giriş ekranı testleri");
    }
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
