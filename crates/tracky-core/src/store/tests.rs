use super::sessions::FOREIGN_LIVE_WINDOW_MS;
use super::*;
use crate::classify::{Classifier, TagKind};
use crate::classify::{Client, DEFAULT_CATEGORIES, Rule, RuleField, Tag};
use crate::model::{IDLE_APP_ID, Session};
use crate::report;
use crate::timesheet::{EntryKind, ProjectMapping, Timesheet, TimesheetConfig, TimesheetEntry};

fn t(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

fn session(app: &str, url: Option<&str>, start: i64, end: i64) -> Session {
    Session {
        id: Uuid::new_v4(),
        app_id: format!("com.test.{app}"),
        app_name: app.into(),
        title: "t".into(),
        url: url.map(Into::into),
        domain: url.and_then(crate::url_util::domain_of),
        started_at: t(start),
        ended_at: t(end),
        category_id: None,
        project_id: None,
        block_from: None,
    }
}

#[test]
fn backups_are_complete_and_can_be_inspected() {
    let dir = std::env::temp_dir().join(format!("tracky-backup-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("kum.db")).unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    let copy = dir.join("yedek.db");
    store.backup_to(&copy).unwrap();
    assert!(
        store.backup_to(&copy).is_err(),
        "var olan dosyanın üzerine yazılmaz"
    );

    let info = Store::inspect_backup(&copy).unwrap();
    assert_eq!(info.sessions, 1);
    assert_eq!(info.last_activity, Some(t(600)));
    let restored = Store::open(&copy).unwrap();
    assert_eq!(restored.device_id(), store.device_id());
    assert_eq!(restored.app_totals(t(0), t(3600)).unwrap().len(), 1);

    let junk = dir.join("not.db");
    std::fs::write(&junk, "merhaba").unwrap();
    assert!(Store::inspect_backup(&junk).is_err());
    let empty = dir.join("bos.db");
    Connection::open(&empty).unwrap();
    assert!(Store::inspect_backup(&empty).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_file_finds_damaged_pages() {
    let dir = std::env::temp_dir().join(format!("tracky-check-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kum.db");
    {
        let store = Store::open(&path).unwrap();
        for i in 0..300 {
            store
                .upsert_session(&session("A", None, i * 60, i * 60 + 30))
                .unwrap();
        }
        store
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    assert_eq!(Store::check_file(&path).unwrap(), None);

    // İlk sayfadan sonraki sayfaların başına çöp yaz (yarım kalan disk yazmaları gibi). Sayfanın
    // boş kısmına yazılan çöp denetimde görünmeyebilir; sayfa başlığı ise her zaman okunur.
    let mut bytes = std::fs::read(&path).unwrap();
    for page in bytes.chunks_mut(4096).skip(1) {
        page[..16].fill(0xAB);
    }
    std::fs::write(&path, bytes).unwrap();
    assert!(!matches!(Store::check_file(&path), Ok(None)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copy_database_includes_wal_contents() {
    let dir = std::env::temp_dir().join(format!("tracky-walcopy-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("kum.db")).unwrap();
    store
        .conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    // Açık veritabanının dosyaları kopyalanır: son oturum yalnızca -wal'da.
    let moved = dir.join("eski");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::copy(dir.join("kum.db"), moved.join("kum.db")).unwrap();
    std::fs::copy(dir.join("kum.db-wal"), moved.join("kum.db-wal")).unwrap();
    drop(store);

    let target = dir.join("kopya.db");
    Store::copy_database(&moved.join("kum.db"), &target).unwrap();
    assert_eq!(Store::inspect_backup(&target).unwrap().sessions, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn checkpoint_file_folds_wal_into_the_database() {
    let dir = std::env::temp_dir().join(format!("tracky-ckpt-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kum.db");
    let store = Store::open(&path).unwrap();
    store
        .conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    // Çökme gibi: bağlantı kapanmadan dosyalar kenara alınır.
    let crashed = dir.join("cokme.db");
    std::fs::copy(&path, &crashed).unwrap();
    std::fs::copy(dir.join("kum.db-wal"), dir.join("cokme.db-wal")).unwrap();
    drop(store);
    Store::checkpoint_file(&crashed).unwrap();
    let wal = std::fs::metadata(dir.join("cokme.db-wal")).map_or(0, |m| m.len());
    assert_eq!(wal, 0);
    let alone = dir.join("yalniz.db");
    std::fs::copy(&crashed, &alone).unwrap();
    assert_eq!(Store::inspect_backup(&alone).unwrap().sessions, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restored_rows_are_newer_and_unsynced() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    store
        .conn
        .execute_batch(
            "UPDATE sessions SET synced_at = updated_at;
                 UPDATE tags SET synced_at = updated_at;",
        )
        .unwrap();
    store
        .save_setting("sync_cursor:sessions", &"2026-01-01T00:00:00Z")
        .unwrap();
    let before = Utc::now().timestamp_millis();
    store.mark_restored_for_sync().unwrap();
    let (pending, oldest): (i64, i64) = store
        .conn
        .query_row(
            "SELECT COUNT(*), MIN(updated_at) FROM sessions WHERE synced_at IS NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(pending, 1);
    assert!(oldest >= before);
    let synced_tags: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE synced_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(synced_tags, 0);
    assert!(
        store
            .setting::<String>("sync_cursor:sessions")
            .unwrap()
            .is_none()
    );
}

#[test]
fn device_id_persists_across_reopen() {
    let dir = std::env::temp_dir().join(format!("tracky-test-{}.db", Uuid::new_v4()));
    let first = Store::open(&dir).unwrap();
    let sync: i64 = first
        .conn()
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 1, "WAL ile synchronous=NORMAL");
    let first = first.device_id();
    let second = Store::open(&dir).unwrap().device_id();
    assert_eq!(first, second);
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn upsert_updates_in_progress_session() {
    let store = Store::open_in_memory().unwrap();
    let mut s = session("Code", None, 0, 10);
    store.upsert_session(&s).unwrap();
    s.ended_at = t(60);
    store.upsert_session(&s).unwrap();
    let all = store.sessions_between(t(0), t(100)).unwrap();
    assert_eq!(all, vec![s]);
}

#[test]
fn totals_are_clipped_to_range() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_session(&session("Code", None, 0, 100))
        .unwrap();
    store
        .upsert_session(&session("Code", None, 200, 300))
        .unwrap();
    store
        .upsert_session(&session("Chrome", Some("https://github.com/x"), 100, 200))
        .unwrap();
    store
        .upsert_session(&session("Chrome", Some("youtube.com/watch"), 300, 330))
        .unwrap();

    // [50, 250) aralığı: Code 50 + 50, Chrome 100.
    let apps = store.app_totals(t(50), t(250)).unwrap();
    let got: Vec<_> = apps.iter().map(|u| (u.label.as_str(), u.seconds)).collect();
    assert_eq!(got, [("Chrome", 100), ("Code", 100)]);

    let domains = store.domain_totals(t(0), t(1000)).unwrap();
    let got: Vec<_> = domains
        .iter()
        .map(|u| (u.key.as_str(), u.seconds))
        .collect();
    assert_eq!(got, [("github.com", 100), ("youtube.com", 30)]);
}

#[test]
fn privacy_settings_round_trip() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(
        store.privacy_settings().unwrap(),
        PrivacySettings::default()
    );
    let s = PrivacySettings {
        excluded_apps: vec!["com.1password.1password".into()],
        ..Default::default()
    };
    store.save_privacy_settings(&s).unwrap();
    store.save_privacy_settings(&s).unwrap();
    assert_eq!(store.privacy_settings().unwrap(), s);
}

#[test]
fn title_totals_for_one_app() {
    let store = Store::open_in_memory().unwrap();
    let mut a = session("Safari", None, 0, 60);
    a.title = "GitHub".into();
    let mut b = session("Safari", None, 60, 90);
    b.title = "Gmail".into();
    let mut c = session("Safari", None, 90, 150);
    c.title = "GitHub".into();
    // Başka uygulama sayılmaz (çakışan oturum başka cihaz demek olurdu, bu yüzden sonra).
    for s in [&a, &b, &c, &session("Code", None, 150, 500)] {
        store.upsert_session(s).unwrap();
    }
    let got: Vec<_> = store
        .title_totals("com.test.Safari", t(0), t(1000))
        .unwrap()
        .into_iter()
        .map(|u| (u.label, u.seconds))
        .collect();
    assert_eq!(
        got,
        [("GitHub".to_string(), 120), ("Gmail".to_string(), 30)]
    );
}

#[test]
fn title_totals_count_device_overlaps_once() {
    let store = Store::open_in_memory().unwrap();
    let mut a = session("Safari", None, 0, 60);
    a.title = "GitHub".into();
    let mut b = session("Safari", None, 30, 90);
    b.title = "GitHub".into();
    store.upsert_session(&a).unwrap();
    store.upsert_session(&b).unwrap();
    // İkinci oturum başka bilgisayardan: 30–60 arası iki kez sayılmamalı.
    store
        .conn()
        .execute(
            "UPDATE sessions SET device_id = ?1 WHERE id = ?2",
            params![Uuid::new_v4().to_string(), b.id.to_string()],
        )
        .unwrap();
    let got = store
        .title_totals("com.test.Safari", t(0), t(1000))
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].seconds, 90);
}

#[test]
fn tag_totals_sum_milliseconds_before_rounding() {
    let store = Store::open_in_memory().unwrap();
    let dev = store
        .tags()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "Geliştirme")
        .unwrap();
    // Üç 1,5 saniyelik oturum 4,5 saniyedir (→ 4); tek tek kırpılsa 3 olurdu.
    for i in 0..3 {
        let mut s = session("Code", None, i * 10, i * 10);
        s.app_id = "com.microsoft.VSCode".into();
        s.ended_at = s.started_at + chrono::Duration::milliseconds(1500);
        store.upsert_session(&s).unwrap();
    }
    let totals = store.category_totals(t(0), t(1000)).unwrap();
    assert_eq!(totals.get(&dev.id), Some(&4));
}

#[test]
fn seeds_default_categories_once() {
    let store = Store::open_in_memory().unwrap();
    let tags = store.tags().unwrap();
    assert_eq!(tags.len(), DEFAULT_CATEGORIES.len());
    assert!(!store.rules().unwrap().is_empty());
    // Silinen varsayılan kategori yeniden eklenmez.
    store.delete_tag(&tags[0].id).unwrap();
    store.seed_default_tags().unwrap();
    assert_eq!(store.tags().unwrap().len(), DEFAULT_CATEGORIES.len() - 1);

    // Varsayılanlar en eski sürüm olarak tohumlanır.
    let max: i64 = store
        .conn
        .query_row(
            "SELECT MAX(updated_at) FROM tags WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(max, 0);

    // İki kurulum aynı varsayılan kimlikleri üretir.
    let other = Store::open_in_memory().unwrap();
    let ids = |s: &Store| {
        s.tags()
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };
    assert!(ids(&store).iter().all(|id| ids(&other).contains(id)));
}

#[test]
fn assign_app_category_replaces_existing_rule() {
    let store = Store::open_in_memory().unwrap();
    let tags = store.tags().unwrap();
    let comm = tags.iter().find(|t| t.name == "İletişim").unwrap();
    let dev = tags.iter().find(|t| t.name == "Geliştirme").unwrap();
    let mut s = session("x", None, 0, 60);
    s.app_id = "com.microsoft.teams2".into();
    store.upsert_session(&s).unwrap();

    let category = |store: &Store| {
        let r = store.report(t(0), t(100), &[t(0)], false).unwrap();
        r.apps[0].category_id.clone()
    };
    assert_eq!(category(&store).as_deref(), Some(comm.id.as_str()));
    store
        .assign_app_category("com.microsoft.teams2", Some(&dev.id))
        .unwrap();
    assert_eq!(category(&store).as_deref(), Some(dev.id.as_str()));
    store
        .assign_app_category("com.microsoft.teams2", None)
        .unwrap();
    assert_eq!(category(&store), None);
}

#[test]
fn edits_refuse_another_devices_live_session() {
    let now = Utc::now();
    let mut live = session("Code", None, 0, 0);
    live.started_at = now - chrono::Duration::hours(1);
    live.ended_at = now;
    let (from, to) = (
        now - chrono::Duration::minutes(30),
        now - chrono::Duration::minutes(20),
    );

    // Kendi cihazının süren oturumu düzenlenebilir.
    let own = Store::open_in_memory().unwrap();
    own.upsert_session(&live).unwrap();
    assert_eq!(own.delete_between(from, to).unwrap(), 1);

    // Başka cihazınki reddedilir ve hiçbir şey bölünmez.
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&live).unwrap();
    store
        .conn()
        .execute(
            "UPDATE sessions SET device_id = ?1",
            params![Uuid::new_v4().to_string()],
        )
        .unwrap();
    for result in [
        store.set_category_between(from, to, None),
        store.set_project_between(from, to, None),
        store.delete_between(from, to),
    ] {
        assert!(matches!(result, Err(StoreError::ForeignLiveSession)));
    }
    assert_eq!(store.sessions_between(from, to).unwrap().len(), 1);

    // Bittikten (pencere geçtikten) sonra düzenlenebilir.
    let ended = ms(now) - FOREIGN_LIVE_WINDOW_MS - 1;
    store
        .conn()
        .execute("UPDATE sessions SET ended_at = ?1", params![ended])
        .unwrap();
    assert_eq!(store.delete_between(from, to).unwrap(), 1);
}

#[test]
fn assign_app_category_keeps_prefix_rules_for_other_apps() {
    let store = Store::open_in_memory().unwrap();
    let tags = store.tags().unwrap();
    let comm = tags.iter().find(|t| t.name == "İletişim").unwrap();
    let dev = tags.iter().find(|t| t.name == "Geliştirme").unwrap();
    store
        .assign_app_category("com.jetbrains.pycharm", Some(&comm.id))
        .unwrap();
    let classifier = Classifier::new(&store.tags().unwrap(), &store.rules().unwrap());
    assert_eq!(
        classifier.app_category("com.jetbrains.pycharm").as_deref(),
        Some(comm.id.as_str())
    );
    assert_eq!(
        classifier.app_category("com.jetbrains.intellij").as_deref(),
        Some(dev.id.as_str())
    );
}

#[test]
fn search_export_keeps_matching_sessions_clipped_to_range() {
    let store = Store::open_in_memory().unwrap();
    let mut hit = session("Code", None, 0, 7200);
    hit.title = "sync.rs — Tracky".into();
    let miss = session("Slack", None, 0, 600);
    store.upsert_session(&hit).unwrap();
    store.upsert_session(&miss).unwrap();
    let csv = store
        .export_search_csv("tracky", t(3600), t(10_000))
        .unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains(",3600,Code,"), "{}", rows[0]);
    assert_eq!(
        store
            .export_search_csv("  ", t(0), t(10_000))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn known_apps_are_listed_by_last_use_with_latest_name() {
    let store = Store::open_in_memory().unwrap();
    let mut a_old = session("a", None, 0, 60);
    a_old.app_name = "Eski Ad".into();
    let mut a_new = session("a", None, 500, 560);
    a_new.app_name = "Yeni Ad".into();
    let b = session("b", None, 200, 260);
    let deleted = session("c", None, 900, 960);
    for s in [&a_old, &a_new, &b, &deleted] {
        store.upsert_session(s).unwrap();
    }
    store.delete_session(&deleted.id).unwrap();
    let apps: Vec<_> = store
        .known_apps(10)
        .unwrap()
        .into_iter()
        .map(|u| (u.key, u.label))
        .collect();
    assert_eq!(
        apps,
        [
            ("com.test.a".to_string(), "Yeni Ad".to_string()),
            ("com.test.b".to_string(), "b".to_string())
        ]
    );
    assert_eq!(store.known_apps(1).unwrap().len(), 1);
}

#[test]
fn timesheet_rows_can_be_edited_added_and_exported() {
    let store = Store::open_in_memory().unwrap();
    let mut work = session("Figma", None, 0, 3600);
    work.title = "Trumore Loyalty UI/UX — Figma".into();
    store.upsert_session(&work).unwrap();
    let p = store.accept_project_suggestion("Trumore").unwrap();
    let sheet = Timesheet {
        id: "togg".into(),
        default_party: "ADBA".into(),
        projects: vec![ProjectMapping {
            project_id: p.id.clone(),
            division: String::new(),
            party: None,
            default_details: None,
        }],
        ..Default::default()
    };
    store
        .save_timesheet_config(&TimesheetConfig {
            timesheets: vec![sheet.clone()],
            ..Default::default()
        })
        .unwrap();
    let ctx = store.timesheet_context().unwrap();
    let (from, to) = (t(-36_000), t(36_000));
    let pieces = store.timesheet_pieces(&ctx, from, to, &[]).unwrap();
    let day = chrono::DateTime::<chrono::Local>::from(t(0)).date_naive();
    let rows = |store: &Store| {
        let pieces = store.timesheet_pieces(&ctx, from, to, &[]).unwrap();
        store
            .timesheet_day(&ctx, &sheet, day, &pieces)
            .unwrap()
            .rows
    };
    let proposed = store
        .timesheet_day(&ctx, &sheet, day, &pieces)
        .unwrap()
        .rows;
    assert_eq!(proposed.len(), 1);
    assert_eq!(
        (
            proposed[0].entry.division.as_str(),
            proposed[0].entry.party.as_str()
        ),
        ("Trumore", "ADBA")
    );
    assert_eq!(proposed[0].entry.project_id, p.id);
    assert_eq!(proposed[0].id, None, "düzenlenene kadar canlı öneri");

    // Düzenle (kaydedilir), elle satır ekle.
    let mut edited = proposed[0].entry.clone();
    edited.details = "Loyalty ekranları".into();
    let id = store.save_timesheet_entry(None, &edited).unwrap();
    let mut extra = edited.clone();
    extra.kind = EntryKind::F2F;
    extra.hours = 0.5;
    extra.coverage = None;
    let extra_id = store.save_timesheet_entry(None, &extra).unwrap();
    assert_eq!(store.timesheet_entries(day, day).unwrap().len(), 2);
    assert!(
        store
            .save_timesheet_entry(
                None,
                &TimesheetEntry {
                    hours: 0.0,
                    ..extra.clone()
                }
            )
            .is_err()
    );

    // Aktarılan kayıt korunur: değiştirilemez, gün yeniden önerilince silinmez ve işi
    // ikinci kez önerilmez (dosyaya iki kez yazılırdı).
    store
        .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), &sheet.id, "")
        .unwrap();
    assert!(store.save_timesheet_entry(Some(&id), &edited).is_err());
    assert_eq!(store.reset_timesheet_day(&sheet, day).unwrap(), 1);
    let left = rows(&store);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].entry.details, "Loyalty ekranları");
    assert!(left[0].exported);
    assert!(
        left.iter()
            .all(|e| e.id.as_deref() != Some(extra_id.as_str()))
    );
}

#[test]
fn report_counts_overlapping_devices_once() {
    let store = Store::open_in_memory().unwrap();
    // İki bilgisayar: 0–3600 dizüstü, 1800–2400 masaüstü (eşitlemeyle gelmiş gibi).
    store
        .upsert_session(&session("Video", None, 0, 3600))
        .unwrap();
    store
        .upsert_session(&session("Figma", None, 1800, 2400))
        .unwrap();
    let r = store.report(t(0), t(3600), &[t(0)], false).unwrap();
    assert_eq!(r.total_seconds, 3600);
    let figma = r.apps.iter().find(|a| a.app_name == "Figma").unwrap();
    assert_eq!(figma.seconds, 600);
    // Ham kayıtlar değişmez.
    assert_eq!(store.sessions_between(t(0), t(3600)).unwrap().len(), 2);
}

#[test]
fn projects_link_to_clients() {
    let store = Store::open_in_memory().unwrap();
    let p = store.accept_project_suggestion("Trumore").unwrap();
    let togg = Client {
        id: Uuid::new_v4().to_string(),
        name: " Togg ".into(),
    };
    store.upsert_client(&togg, 0).unwrap();
    assert_eq!(store.clients().unwrap()[0].name, "Togg");
    store.set_project_client(&p.id, Some(&togg.id)).unwrap();
    assert_eq!(store.project_clients().unwrap().get(&p.id), Some(&togg.id));
    // Projeyi yeniden adlandırmak müşterisini bozmaz.
    store
        .upsert_tag(
            &Tag {
                name: "Trumore 2".into(),
                ..p.clone()
            },
            0,
        )
        .unwrap();
    assert_eq!(store.project_clients().unwrap().get(&p.id), Some(&togg.id));
    // Olmayan müşteri ya da kategori bağlanamaz; boş ad kaydedilmez.
    assert!(store.set_project_client(&p.id, Some("yok")).is_err());
    let cat = store.tags().unwrap()[0].id.clone();
    assert!(store.set_project_client(&cat, Some(&togg.id)).is_err());
    assert!(
        store
            .upsert_client(
                &Client {
                    id: "x".into(),
                    name: " ".into()
                },
                1
            )
            .is_err()
    );
    // Müşterisiz yapılabilir; müşteri silinince proje kalır, bağlantı kalkar.
    store.set_project_client(&p.id, None).unwrap();
    assert!(store.project_clients().unwrap().is_empty());
    store.set_project_client(&p.id, Some(&togg.id)).unwrap();
    store.delete_client(&togg.id).unwrap();
    assert!(store.clients().unwrap().is_empty());
    assert!(store.project_clients().unwrap().is_empty());
    assert!(store.tags().unwrap().iter().any(|t| t.id == p.id));
}

#[test]
fn manual_project_overrides_rules_and_splits_at_range() {
    let store = Store::open_in_memory().unwrap();
    let mut code = session("Code", None, 0, 3600);
    code.title = "sync.rs — tracky".into();
    store.upsert_session(&code).unwrap();
    let kum = store.accept_project_suggestion("tracky").unwrap(); // kural: "tracky"
    let togg = Tag {
        id: Uuid::new_v4().to_string(),
        kind: TagKind::Project,
        name: "Trumore".into(),
        color: 2,
    };
    store.upsert_tag(&togg, 9).unwrap();
    let projects = |from: i64, to: i64| {
        let r = store.report(t(from), t(to), &[t(from)], false).unwrap();
        r.projects
            .into_iter()
            .map(|b| (b.id, b.seconds))
            .collect::<Vec<_>>()
    };
    // Son yarım saat elle Trumore'a: kural yalnızca ilk yarıda kalır.
    assert_eq!(
        store
            .set_project_between(t(1800), t(3600), Some(&togg.id))
            .unwrap(),
        1
    );
    let mut got = projects(0, 3600);
    got.sort();
    let mut want = vec![(Some(kum.id.clone()), 1800), (Some(togg.id.clone()), 1800)];
    want.sort();
    assert_eq!(got, want);
    // Kurallara döndürünce yine tamamı kurala göre.
    store.set_project_between(t(0), t(3600), None).unwrap();
    assert_eq!(projects(0, 3600), [(Some(kum.id.clone()), 3600)]);
    // "Projesiz": kurala uysa da ilk yarım saat hiçbir projeye sayılmaz; geri alınabilir.
    store
        .set_project_between(t(0), t(1800), Some(crate::classify::NO_PROJECT))
        .unwrap();
    let mut got = projects(0, 3600);
    got.sort();
    assert_eq!(got, [(None, 1800), (Some(kum.id.clone()), 1800)]);
    store.set_project_between(t(0), t(3600), None).unwrap();
    assert_eq!(projects(0, 3600), [(Some(kum.id.clone()), 3600)]);
    // Kategori kimliği proje olarak verilemez.
    let cat = store.tags().unwrap()[0].id.clone();
    assert!(store.set_project_between(t(0), t(10), Some(&cat)).is_err());
    // Elle kayıt projeyle eklenebilir.
    let m = store
        .add_manual_session("Toplantı", t(4000), t(5800), None, Some(&togg.id))
        .unwrap();
    assert_eq!(m.project_id.as_deref(), Some(togg.id.as_str()));
    assert_eq!(projects(4000, 5800), [(Some(togg.id.clone()), 1800)]);
}

#[test]
fn long_sessions_are_found_by_range_queries() {
    // Aralık sorguları "en uzun oturum" alt sınırını kullanır; sonradan uzatılan
    // (takip) ya da uzaktan gelen uzun oturumlar da bulunmalı.
    let store = Store::open_in_memory().unwrap();
    let day = 86_400;
    store.upsert_session(&session("kisa", None, 0, 60)).unwrap();
    let mut long = session("uzun", None, 0, 60);
    store.upsert_session(&long).unwrap();
    long.ended_at = t(30 * day);
    store.upsert_session(&long).unwrap();
    let found = store.sessions_between(t(20 * day), t(21 * day)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].app_name, "uzun");
    assert_eq!(
        store.app_totals(t(20 * day), t(21 * day)).unwrap()[0].seconds,
        day
    );
    assert_eq!(store.delete_between(t(20 * day), t(21 * day)).unwrap(), 1);
    assert!(
        store
            .sessions_between(t(20 * day), t(21 * day))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn suggestions_can_be_accepted_or_dismissed() {
    let store = Store::open_in_memory().unwrap();
    let mut code = session("x", None, 0, 40 * 60);
    code.app_id = "com.microsoft.VSCode".into();
    code.title = "sync.rs — tracky".into();
    let mut postman = session("Postman", None, 40 * 60, 60 * 60);
    postman.app_id = "com.postmanlabs.mac".into();
    store.upsert_session(&code).unwrap();
    store.upsert_session(&postman).unwrap();

    let now = t(3600);
    let s = store.suggestions(now).unwrap();
    assert_eq!(s.projects[0].name, "tracky");
    assert_eq!(s.categories[0].label, "Postman");

    let tag = store.accept_project_suggestion("tracky").unwrap();
    let c = &s.categories[0];
    store
        .accept_category_suggestion(c.field, &c.pattern, &c.category_id)
        .unwrap();
    let after = store.suggestions(now).unwrap();
    assert!(after.projects.is_empty() && after.categories.is_empty());
    let report = store.report(t(0), now, &[t(0)], false).unwrap();
    assert_eq!(report.projects[0].id.as_deref(), Some(tag.id.as_str()));

    // Yoksayılan öneri bir daha gelmez.
    let other = Store::open_in_memory().unwrap();
    other.upsert_session(&code).unwrap();
    other.dismiss_suggestion(&s.projects[0].key).unwrap();
    other.dismiss_suggestion(&s.projects[0].key).unwrap();
    assert!(other.suggestions(now).unwrap().projects.is_empty());
}

#[test]
fn setting_updated_at_never_goes_backwards() {
    let store = Store::open_in_memory().unwrap();
    store.save_setting("theme", &"dark").unwrap();
    // Saati ileride olan bir cihazdan gelmiş sürüm.
    let future = ms(Utc::now()) + 3_600_000;
    store
        .conn
        .execute(
            "UPDATE settings SET updated_at = ?1, synced_at = ?1 WHERE key = 'theme'",
            [future],
        )
        .unwrap();
    store.save_setting("theme", &"light").unwrap();
    let after: i64 = store
        .conn
        .query_row(
            "SELECT updated_at FROM settings WHERE key = 'theme'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after > future);
}

#[test]
fn empty_or_reversed_ranges_are_rejected() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    assert!(matches!(
        store.delete_between(t(300), t(300)),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store.delete_between(t(300), t(240)),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn updated_at_never_goes_backwards() {
    let store = Store::open_in_memory().unwrap();
    let tag = store.tags().unwrap()[0].clone();
    // Uzaktan, yerel saatten çok ileri bir sürüm gelmiş olsun.
    let future = ms(Utc::now()) + 3_600_000;
    store
        .conn
        .execute(
            "UPDATE tags SET updated_at = ?2 WHERE id = ?1",
            params![tag.id, future],
        )
        .unwrap();
    store
        .upsert_tag(
            &Tag {
                name: "Yeni".into(),
                ..tag.clone()
            },
            0,
        )
        .unwrap();
    let after: i64 = store
        .conn
        .query_row(
            "SELECT updated_at FROM tags WHERE id = ?1",
            [&tag.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after > future);
}

#[test]
fn generic_settings() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.setting::<bool>("onboarded").unwrap(), None);
    store.save_setting("onboarded", &true).unwrap();
    assert_eq!(store.setting::<bool>("onboarded").unwrap(), Some(true));
}

#[test]
fn rejects_newer_schema() {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    assert!(matches!(Store::init(conn), Err(StoreError::Invalid(_))));
}

#[test]
fn manual_category_overrides_rules_and_can_be_cleared() {
    let store = Store::open_in_memory().unwrap();
    let tags = store.tags().unwrap();
    let design = tags
        .iter()
        .find(|t| t.name == "Tasarım")
        .unwrap()
        .id
        .clone();
    store
        .upsert_session(&session("VSCode", None, 0, 600))
        .unwrap();
    let mut vscode = session("Other", None, 700, 900);
    vscode.app_id = "com.microsoft.VSCode".into();
    store.upsert_session(&vscode).unwrap();

    let report = |s: &Store| s.report(t(0), t(3600), &[t(0)], false).unwrap();
    let dev = report(&store).categories;
    assert!(dev.iter().any(|b| b.id.is_some() && b.seconds == 200));

    // Yalnızca aralıkla kesişen oturum değişir.
    assert_eq!(
        store
            .set_category_between(t(650), t(1000), Some(&design))
            .unwrap(),
        1
    );
    let cats = report(&store).categories;
    assert!(cats.contains(&report::Bucket {
        id: Some(design.clone()),
        seconds: 200
    }));

    // Geri alınınca kurala döner; bilinmeyen etiket reddedilir.
    store.set_category_between(t(650), t(1000), None).unwrap();
    assert!(
        !report(&store)
            .categories
            .iter()
            .any(|b| b.id.as_deref() == Some(&*design))
    );
    assert!(
        store
            .set_category_between(t(0), t(10), Some("yok"))
            .is_err()
    );
}

#[test]
fn unassigned_block_adopts_project_it_grows_over() {
    let store = Store::open_in_memory().unwrap();
    let project = store.accept_project_suggestion("Togg").unwrap();
    let p = Some(project.id.as_str());
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    store
        .upsert_session(&session("B", None, 600, 1200))
        .unwrap();
    store.set_project_between(t(0), t(600), p).unwrap();

    // Projesiz B bloğunun başı projeli A'nın üstüne uzar: ikisi de Togg olur.
    store
        .resize_block(t(600), t(1200), t(0), t(1200), "B", None, None)
        .unwrap();
    let all = store.sessions_between(t(0), t(1200)).unwrap();
    assert!(all.iter().all(|s| s.project_id.as_deref() == p));
    let report = store.report(t(0), t(1200), &[t(0)], true).unwrap();
    assert_eq!(report.work.blocks.len(), 1);
    assert_eq!(report.work.blocks[0].project_id.as_deref(), p);
}

#[test]
fn resize_block_trims_joins_and_fills_gaps() {
    let store = Store::open_in_memory().unwrap();
    let cat = store.tags().unwrap()[0].id.clone();
    let project = store.accept_project_suggestion("Togg").unwrap();
    // Blok 600–1800; önünde başka iş, arkasında boşta süre ve kaydı olmayan boşluk.
    store.upsert_session(&session("B", None, 0, 600)).unwrap();
    store
        .upsert_session(&session("A", None, 600, 1800))
        .unwrap();
    store
        .upsert_session(&Session::idle(t(1800), t(2400)))
        .unwrap();
    let p = Some(project.id.as_str());
    store.set_project_between(t(600), t(1800), p).unwrap();
    store
        .set_category_between(t(600), t(1800), Some(&cat))
        .unwrap();

    // Sonu 3000'e uzar, başı 900'e kısalır.
    store
        .resize_block(t(600), t(1800), t(900), t(3000), "Togg", Some(&cat), p)
        .unwrap();
    let all = store.sessions_between(t(0), t(3600)).unwrap();
    let work: Vec<_> = all.iter().filter(|s| s.counts_as_work()).collect();
    // Kısalan kısım silinmedi: projesiyle ayrı blok oldu; önceki iş yerinde.
    assert!(all.iter().any(|s| s.app_name == "A"
        && (s.started_at, s.ended_at) == (t(600), t(900))
        && s.project_id.as_deref() == p
        && s.block_from.is_none()));
    assert!(
        all.iter()
            .any(|s| s.started_at == t(900) && s.block_from == Some(t(900)))
    );
    let report = store.report(t(0), t(3600), &[t(0)], true).unwrap();
    let spans: Vec<_> = report
        .work
        .blocks
        .iter()
        .filter(|b| b.project_id.as_deref() == p)
        .map(|b| (b.start, b.end))
        .collect();
    assert_eq!(spans, [(t(600), t(900)), (t(900), t(3000))]);
    assert!(
        work.iter()
            .any(|s| s.app_name == "B" && s.ended_at == t(600))
    );
    // Uzayan kısım: boşta süre atanınca çalışma olur, kaydı olmayan boşluk elle kayıtla
    // dolar; hepsi projede ve kategoride.
    let joined: Vec<_> = work.iter().filter(|s| s.started_at >= t(1800)).collect();
    assert_eq!(joined.len(), 2);
    assert!(joined[0].is_idle() && joined[0].ended_at == t(2400));
    assert!(joined[1].is_manual());
    assert_eq!(
        (joined[1].started_at, joined[1].ended_at),
        (t(2400), t(3000))
    );
    let block: i64 = work
        .iter()
        .filter(|s| s.started_at >= t(900))
        .inspect(|s| {
            assert_eq!(s.project_id.as_deref(), p);
            assert_eq!(s.category_id.as_deref(), Some(&*cat));
        })
        .map(|s| (s.ended_at - s.started_at).num_seconds())
        .sum();
    assert_eq!(block, 2100);

    // Başı başka işin üstüne uzayınca o iş ve kesilen parça bloğa katılır: bölme kalkar.
    store
        .resize_block(t(900), t(3000), t(300), t(3000), "Togg", Some(&cat), p)
        .unwrap();
    let b = store.sessions_between(t(300), t(600)).unwrap();
    assert!(
        b.iter()
            .all(|s| s.app_name == "B" && s.project_id.as_deref() == p)
    );
    let all = store.sessions_between(t(0), t(3600)).unwrap();
    assert!(all.iter().all(|s| s.block_from.is_none()));
    let report = store.report(t(0), t(3600), &[t(0)], true).unwrap();
    assert!(
        report
            .work
            .blocks
            .iter()
            .any(|b| (b.start, b.end) == (t(300), t(3000)))
    );

    // Gelecek ve ters aralık reddedilir.
    let later = Utc::now() + chrono::Duration::hours(1);
    assert!(
        store
            .resize_block(t(300), t(3000), t(300), later, "Togg", None, p)
            .is_err()
    );
    assert!(
        store
            .resize_block(t(300), t(3000), t(3000), t(300), "Togg", None, p)
            .is_err()
    );
}

#[test]
fn scoped_range_edits_touch_only_the_chosen_app_or_window() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    store.upsert_session(&session("B", None, 0, 600)).unwrap();
    store
        .upsert_session(&Session {
            title: "u".into(),
            ..session("A", None, 600, 900)
        })
        .unwrap();
    let project = store.accept_project_suggestion("Togg").unwrap();
    let only_a = EditScope {
        app_ids: vec!["com.test.A".into()],
        titles: None,
    };
    // A'nın iki penceresi projeye geçer, aynı dilimdeki B değişmez.
    assert_eq!(
        store
            .set_project_in(t(0), t(900), Some(&project.id), Some(&only_a))
            .unwrap(),
        2
    );
    let projects = |store: &Store| {
        let mut v: Vec<(String, String, Option<String>)> = store
            .sessions_between(t(0), t(3600))
            .unwrap()
            .into_iter()
            .map(|s| (s.app_name, s.title, s.project_id))
            .collect();
        v.sort();
        v
    };
    let p = Some(project.id.clone());
    assert_eq!(
        projects(&store),
        [
            ("A".into(), "t".into(), p.clone()),
            ("A".into(), "u".into(), p.clone()),
            ("B".into(), "t".into(), None),
        ]
    );
    // Yalnızca bir pencere silinir.
    let window = EditScope {
        app_ids: vec!["com.test.A".into()],
        titles: Some(vec!["u".into()]),
    };
    assert_eq!(store.delete_in(t(0), t(900), Some(&window)).unwrap(), 1);
    assert_eq!(projects(&store).len(), 2);
    // Boş kapsam reddedilir (hiçbir şeye dokunmaz değil, hata).
    assert!(
        store
            .delete_in(t(0), t(900), Some(&EditScope::default()))
            .is_err()
    );
}

#[test]
fn delete_between_and_manual_sessions() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    store.upsert_session(&session("B", None, 600, 900)).unwrap();
    assert_eq!(store.delete_between(t(0), t(600)).unwrap(), 1);
    assert_eq!(store.app_totals(t(0), t(3600)).unwrap().len(), 1);

    // Çakışan elle kayıt reddedilir, boş aralığa eklenir.
    assert!(
        store
            .add_manual_session("Toplantı", t(800), t(1200), None, None)
            .is_err()
    );
    assert!(
        store
            .add_manual_session("  ", t(1000), t(1200), None, None)
            .is_err()
    );
    let cat = store.tags().unwrap()[0].id.clone();
    let s = store
        .add_manual_session("Toplantı", t(1000), t(1600), Some(&cat), None)
        .unwrap();
    let back = store.sessions_between(t(0), t(3600)).unwrap();
    let manual = back.iter().find(|x| x.id == s.id).unwrap();
    assert!(manual.is_manual());
    assert_eq!(manual.app_name, "Toplantı");
    assert_eq!(manual.category_id.as_deref(), Some(&*cat));
}

#[test]
fn idle_time_is_left_out_of_totals_until_assigned() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&session("A", None, 0, 600)).unwrap();
    store
        .upsert_session(&Session::idle(t(600), t(2400)))
        .unwrap();
    let (from, to) = (t(0), t(3600));
    assert_eq!(
        store.report(from, to, &[from], true).unwrap().total_seconds,
        600
    );
    assert_eq!(store.app_totals(from, to).unwrap().len(), 1);
    assert!(
        store
            .known_apps(10)
            .unwrap()
            .iter()
            .all(|a| a.key != IDLE_APP_ID)
    );
    assert!(
        !store
            .export_csv()
            .unwrap()
            .contains(crate::model::IDLE_NAME)
    );

    // Bir kısmı projeye atanınca o kısım çalışma olur, kalanı boşta kalır.
    let project = store.accept_project_suggestion("Togg").unwrap();
    store
        .set_project_between(t(600), t(1200), Some(&project.id))
        .unwrap();
    let r = store.report(from, to, &[from], true).unwrap();
    assert_eq!(r.total_seconds, 1200);
    assert_eq!(r.idle_seconds, 1200);
    assert_eq!(store.project_totals(from, to).unwrap()[&project.id], 600);
}

#[test]
fn manual_entry_replaces_idle_time() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_session(&Session::idle(t(0), t(3600))).unwrap();
    store
        .add_manual_session("Toplantı", t(600), t(1800), None, None)
        .unwrap();
    let all = store.sessions_between(t(0), t(3600)).unwrap();
    let spans: Vec<_> = all
        .iter()
        .map(|s| (s.is_idle(), s.started_at, s.ended_at))
        .collect();
    assert_eq!(
        spans,
        [
            (true, t(0), t(600)),
            (false, t(600), t(1800)),
            (true, t(1800), t(3600))
        ]
    );
    // Projeye atanmış boşta süre ise elle kaydın üstüne yazılmaz.
    let project = store.accept_project_suggestion("Togg").unwrap();
    store
        .set_project_between(t(0), t(600), Some(&project.id))
        .unwrap();
    assert!(
        store
            .add_manual_session("Okuma", t(0), t(300), None, None)
            .is_err()
    );
}

#[test]
fn domain_rules_are_normalized_and_classify_by_url() {
    let store = Store::open_in_memory().unwrap();
    let project = store.accept_project_suggestion("Togg").unwrap();
    let rule = |pattern: &str| Rule {
        id: Uuid::new_v4().to_string(),
        tag_id: project.id.clone(),
        field: RuleField::Domain,
        pattern: pattern.into(),
    };
    store
        .upsert_rule(&rule("https://www.Jira.Togg.com/"))
        .unwrap();
    assert!(store.upsert_rule(&rule("toplantı notları")).is_err());
    assert!(
        store
            .rules()
            .unwrap()
            .iter()
            .any(|r| r.field == RuleField::Domain && r.pattern == "jira.togg.com")
    );
    store
        .upsert_session(&session(
            "Safari",
            Some("https://jira.togg.com/browse/T-1"),
            0,
            600,
        ))
        .unwrap();
    assert_eq!(
        store.project_totals(t(0), t(3600)).unwrap()[&project.id],
        600
    );
}

#[test]
fn unknown_rule_fields_from_newer_versions_are_skipped() {
    let store = Store::open_in_memory().unwrap();
    let before = store.rules().unwrap().len();
    let tag = store.tags().unwrap()[0].id.clone();
    // Bu sürümün tanımadığı bir tür (CHECK'i atlatmak için doğrudan yazılır).
    store
        .conn()
        .execute_batch(&format!(
            "PRAGMA ignore_check_constraints = ON;
                 INSERT INTO rules (id, tag_id, field, pattern, updated_at)
                 VALUES ('x', '{tag}', 'gelecek', 'p', 0);
                 PRAGMA ignore_check_constraints = OFF;"
        ))
        .unwrap();
    assert_eq!(store.rules().unwrap().len(), before);
}

#[test]
fn range_edits_split_sessions_at_the_boundaries() {
    let store = Store::open_in_memory().unwrap();
    let long = session("A", None, 0, 3000);
    store.upsert_session(&long).unwrap();
    // Ortadaki 1000–2000 silinir; baş ve son korunur, asıl kimlik sonda kalır.
    assert_eq!(store.delete_between(t(1000), t(2000)).unwrap(), 1);
    let left = store.sessions_between(t(0), t(4000)).unwrap();
    let spans: Vec<_> = left.iter().map(|s| (s.started_at, s.ended_at)).collect();
    assert_eq!(spans, [(t(0), t(1000)), (t(2000), t(3000))]);
    assert_eq!(left[1].id, long.id);

    // Kategori ataması da yalnızca aralığın içine uygulanır.
    let cat = store.tags().unwrap()[0].id.clone();
    store
        .set_category_between(t(2500), t(2600), Some(&cat))
        .unwrap();
    let all = store.sessions_between(t(0), t(4000)).unwrap();
    let tagged: Vec<_> = all
        .iter()
        .filter(|s| s.category_id.is_some())
        .map(|s| (s.started_at, s.ended_at))
        .collect();
    assert_eq!(tagged, [(t(2500), t(2600))]);
    assert_eq!(all.len(), 4);
}

#[test]
fn running_session_resumes_after_its_block_is_deleted() {
    let store = Store::open_in_memory().unwrap();
    let mut running = session("A", None, 0, 600);
    store.upsert_session(&running).unwrap();
    store.delete_between(t(0), t(600)).unwrap();
    assert!(store.sessions_between(t(0), t(4000)).unwrap().is_empty());
    // Motor aynı oturumu uzatmaya devam eder: silinme anından itibaren geri gelir.
    let deleted_at: i64 = store
        .conn
        .query_row(
            "SELECT deleted_at FROM sessions WHERE id = ?1",
            [running.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    running.ended_at = from_ms(deleted_at) + chrono::Duration::seconds(30);
    store.upsert_session(&running).unwrap();
    let back = store
        .sessions_between(t(0), from_ms(deleted_at + 60_000))
        .unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].started_at, from_ms(deleted_at));
}
