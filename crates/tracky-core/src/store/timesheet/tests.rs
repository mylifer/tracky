use super::*;
use crate::classify::{Client, Tag};
use crate::model::Session;
use crate::store::Unsent;
use chrono::{Duration, Local, TimeZone};

/// 2026-03-02 (yerel) 09:00'dan `min` dakika sonra.
fn t(min: i64) -> DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 3, 2, 9, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
        + Duration::minutes(min)
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
}

fn work(store: &Store, title: &str, from: i64, to: i64, project: Option<&str>) {
    let s = Session {
        id: Uuid::new_v4(),
        app_id: "com.figma.Desktop".into(),
        app_name: "Figma".into(),
        title: title.into(),
        url: None,
        domain: None,
        started_at: t(from),
        ended_at: t(to),
        category_id: None,
        project_id: None,
        block_from: None,
        passive: false,
    };
    store.upsert_session(&s).unwrap();
    if let Some(p) = project {
        store.set_project_between(t(from), t(to), Some(p)).unwrap();
    }
}

fn project(store: &Store, id: &str, name: &str) {
    store
        .upsert_tag(
            &Tag {
                id: id.into(),
                kind: TagKind::Project,
                name: name.into(),
                color: 1,
            },
            0,
        )
        .unwrap();
}

fn sheet(id: &str, projects: &[&str]) -> Timesheet {
    Timesheet {
        id: id.into(),
        company: id.into(),
        default_party: "ADBA".into(),
        sheet_url: Some("https://script.google.com/macros/s/x/exec".into()),
        projects: projects
            .iter()
            .map(|p| ProjectMapping {
                project_id: (*p).into(),
                division: String::new(),
                party: None,
                default_details: None,
            })
            .collect(),
        ..Default::default()
    }
}

/// Günün satırları: (kimlik var mı, başlangıç, gerçek dakika, açıklama).
fn rows(store: &Store, sheet: &Timesheet) -> (Vec<DayRow>, usize) {
    let ctx = store.timesheet_context().unwrap();
    let pieces = store.timesheet_pieces(&ctx, t(-540), t(900), &[]).unwrap();
    let d = store.timesheet_day(&ctx, sheet, day(), &pieces).unwrap();
    (d.rows, d.hidden)
}

fn short(rows: &[DayRow]) -> Vec<(bool, String, i64, String)> {
    rows.iter()
        .map(|r| {
            (
                r.id.is_some(),
                r.entry.start.format("%H:%M").to_string(),
                (r.entry.worked() * 60.0).round() as i64,
                r.entry.details.clone(),
            )
        })
        .collect()
}

#[test]
fn rows_are_live_until_touched_and_later_work_is_added() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "kum", "Kum");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Kum geliştirme", 60, 120, Some("kum"));
    work(&store, "Rapor", 180, 240, Some("togg"));
    let togg = sheet("togg", &["togg"]);

    // Yalnızca Togg'un işi; hepsi canlı öneri.
    let (r, _) = rows(&store, &togg);
    assert_eq!(
        short(&r),
        [
            (false, "09:00".into(), 60, "Loyalty".into()),
            (false, "12:00".into(), 60, "Rapor".into())
        ]
    );
    // Açıklama düzenlenince satır kaydedilir; anahtarı değişmez.
    let key = r[1].key.clone();
    let id = store
        .save_timesheet_entry(
            None,
            &TimesheetEntry {
                details: "Aylık rapor".into(),
                ..r[1].entry.clone()
            },
        )
        .unwrap();
    // Eski listeyle ikinci kez kaydedilirse aynı satır güncellenir.
    let again = store
        .save_timesheet_entry(
            None,
            &TimesheetEntry {
                details: "Aylık rapor v2".into(),
                ..r[1].entry.clone()
            },
        )
        .unwrap();
    assert_eq!(again, id);
    let (r, _) = rows(&store, &togg);
    assert_eq!(r[1].id.as_deref(), Some(id.as_str()));
    assert_eq!(r[1].key, key);
    assert_eq!(r[1].entry.details, "Aylık rapor v2");

    // Raporda sonradan Togg'a atanan iş yeni satır olur; kaydedilene dokunulmaz.
    work(&store, "Toplantı notu", 250, 280, Some("togg"));
    work(&store, "Analiz", -120, -60, Some("togg"));
    let (r, _) = rows(&store, &togg);
    assert_eq!(
        short(&r),
        [
            (false, "07:00".into(), 60, "Analiz".into()),
            (false, "09:00".into(), 60, "Loyalty".into()),
            (true, "12:00".into(), 60, "Aylık rapor v2".into()),
            (false, "13:10".into(), 30, "Toplantı notu".into())
        ]
    );

    // Kum'un çizelgesinde yalnızca Kum.
    let (r, _) = rows(&store, &sheet("kisisel", &["kum"]));
    assert_eq!(
        short(&r),
        [(false, "10:00".into(), 60, "Kum geliştirme".into())]
    );
}

#[test]
fn dismissed_rows_stay_hidden_and_come_back() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store.dismiss_timesheet_entry(None, &r[0].entry).unwrap();
    let (r, hidden) = rows(&store, &togg);
    assert!(r.is_empty());
    assert_eq!(hidden, 1);
    // Gizleme geri alınınca (silinince) öneri yeniden gelir.
    store.delete_timesheet_entry(&id).unwrap();
    assert_eq!(rows(&store, &togg).0.len(), 1);
    // Kaydedilmiş satır gizlenip geri getirilir.
    let (r, _) = rows(&store, &togg);
    let id = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    store
        .dismiss_timesheet_entry(Some(&id), &r[0].entry)
        .unwrap();
    assert_eq!(rows(&store, &togg).1, 1);
    assert_eq!(
        store.restore_hidden(&togg, day()).unwrap(),
        vec![id.clone()]
    );
    let (r, hidden) = rows(&store, &togg);
    assert_eq!((r.len(), hidden), (1, 0));
    assert_eq!(r[0].id.as_deref(), Some(id.as_str()));
    // Gizlenen satır başka yerde (öneri modeli, müşteri raporu) sayılmaz.
    store
        .dismiss_timesheet_entry(Some(&id), &r[0].entry)
        .unwrap();
    assert!(store.timesheet_entries(day(), day()).unwrap().is_empty());
}

#[test]
fn assigning_time_again_brings_back_a_deleted_rows_work() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "kum", "Kum");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store.dismiss_timesheet_entry(None, &r[0].entry).unwrap();
    assert!(rows(&store, &togg).0.is_empty());
    // Başka projeye atamak silinmiş satıra dokunmaz.
    store.set_project_between(t(0), t(10), Some("kum")).unwrap();
    let spans = |store: &Store| store.timesheet_entry(&id).unwrap().unwrap().entry.spans();
    assert_eq!(spans(&store), vec![(t(0), t(60))]);
    store.set_project_between(t(0), t(10), None).unwrap();
    // Raporda yeniden projeye atanan yarım saat geri gelir; kalanı silinmiş kalır.
    store
        .set_project_between(t(30), t(60), Some("togg"))
        .unwrap();
    let (r, hidden) = rows(&store, &togg);
    assert_eq!((r.len(), hidden), (1, 1));
    assert_eq!(r[0].entry.spans(), vec![(t(30), t(60))]);
    // Bütün aralık atanınca silinmiş satır kalmaz.
    store
        .set_project_between(t(0), t(60), Some("togg"))
        .unwrap();
    let (r, hidden) = rows(&store, &togg);
    assert_eq!((r.len(), hidden), (1, 0));
    assert_eq!(r[0].entry.spans(), vec![(t(0), t(60))]);
}

#[test]
fn a_deleted_meeting_leaves_its_time_to_an_overlapping_one() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    let togg = sheet("togg", &["togg"]);
    let meeting = |uid: &str, subject: &str, from: i64, to: i64, online: bool| Meeting {
        uid: uid.into(),
        start: t(from),
        end: t(to),
        subject: subject.into(),
        online,
        ..Meeting::default()
    };
    let meetings = [
        meeting("plan", "Haftalık plan", 0, 60, true),
        meeting("ux", "Charging UX", 30, 60, false),
    ];
    store.assign_meeting("plan", Some("togg")).unwrap();
    store.assign_meeting("ux", Some("togg")).unwrap();
    let rows = |store: &Store| {
        let ctx = store.timesheet_context().unwrap();
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
            .unwrap();
        let d = store.timesheet_day(&ctx, &togg, day(), &pieces).unwrap();
        d.rows
            .into_iter()
            .map(|r| ((r.entry.worked() * 60.0).round() as i64, r.entry.details))
            .collect::<Vec<_>>()
    };
    // Önce başlayan toplantı çakışan yarım saati alır.
    assert_eq!(rows(&store), [(60, "Haftalık plan".to_string())]);
    // Planlama çizelgeden silindi (yapılmadı): süresi ikinci toplantıya kalır.
    let ctx = store.timesheet_context().unwrap();
    let pieces = store
        .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
        .unwrap();
    let r = store
        .timesheet_day(&ctx, &togg, day(), &pieces)
        .unwrap()
        .rows;
    store.dismiss_timesheet_entry(None, &r[0].entry).unwrap();
    assert_eq!(rows(&store), [(30, "Charging UX".to_string())]);
    // Kalan toplantı, silinen satırın aralığına düşse de kaydedilir.
    let pieces = store
        .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
        .unwrap();
    let r = store
        .timesheet_day(&ctx, &togg, day(), &pieces)
        .unwrap()
        .rows;
    store.save_timesheet_entry(None, &r[0].entry).unwrap();
    assert_eq!(rows(&store), [(30, "Charging UX".to_string())]);
    // Aralıkları bilinmeyen eski satır da aynı saatte başlayan toplantısıyla eşlenir.
    store
        .conn
        .execute("UPDATE timesheet_entries SET coverage = NULL", [])
        .unwrap();
    assert_eq!(rows(&store), [(30, "Charging UX".to_string())]);
}

#[test]
fn work_in_a_deleted_meeting_can_be_saved() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    let togg = sheet("togg", &["togg"]);
    let meetings = [Meeting {
        uid: "ux".into(),
        start: t(30),
        end: t(60),
        subject: "UX gösterimi".into(),
        online: true,
        ..Meeting::default()
    }];
    store.assign_meeting("ux", Some("togg")).unwrap();
    work(&store, "Loyalty", 0, 90, Some("togg"));
    let rows = |store: &Store| {
        let ctx = store.timesheet_context().unwrap();
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
            .unwrap();
        store
            .timesheet_day(&ctx, &togg, day(), &pieces)
            .unwrap()
            .rows
    };
    let r = rows(&store);
    let meeting = r
        .iter()
        .find(|r| r.entry.details == "UX gösterimi")
        .unwrap();
    store.dismiss_timesheet_entry(None, &meeting.entry).unwrap();
    // Toplantı yapılmadı: saati işe kalır ve iş tek satır olarak kaydedilir.
    let r = rows(&store);
    assert_eq!(short(&r), [(false, "09:00".into(), 90, "Loyalty".into())]);
    let id = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    let r = rows(&store);
    assert_eq!(short(&r), [(true, "09:00".into(), 90, "Loyalty".into())]);
    assert_eq!(r[0].id.as_deref(), Some(id.as_str()));
    // Toplantı geri getirilince işi zaten satırda olan saati ikinci kez yazmaz.
    assert_eq!(
        store.restore_hidden(&togg, day()).unwrap().len(),
        1,
        "geri getirilir"
    );
    let r = rows(&store);
    assert_eq!(short(&r), [(true, "09:00".into(), 90, "Loyalty".into())]);
    assert_eq!(
        store.timesheet_entries(day(), day()).unwrap().len(),
        1,
        "işi tamamen satırda olan toplantı gizli kalır: {r:?}"
    );
}

#[test]
fn a_restored_row_leaves_saved_work_alone() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    let togg = sheet("togg", &["togg"]);
    let meetings = [Meeting {
        uid: "ux".into(),
        start: t(30),
        end: t(90),
        subject: "UX gösterimi".into(),
        online: true,
        ..Meeting::default()
    }];
    store.assign_meeting("ux", Some("togg")).unwrap();
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let rows = |store: &Store| {
        let ctx = store.timesheet_context().unwrap();
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
            .unwrap();
        store
            .timesheet_day(&ctx, &togg, day(), &pieces)
            .unwrap()
            .rows
    };
    let r = rows(&store);
    let meeting = r
        .iter()
        .find(|r| r.entry.details == "UX gösterimi")
        .unwrap();
    let hidden = store.dismiss_timesheet_entry(None, &meeting.entry).unwrap();
    // Toplantının ilk yarım saati iş olarak kaydedildi; toplantı geri gelince kalan yarım saat.
    let r = rows(&store);
    assert_eq!(short(&r), [(false, "09:00".into(), 60, "Loyalty".into())]);
    store.save_timesheet_entry(None, &r[0].entry).unwrap();
    store
        .undismiss_timesheet_entries(std::slice::from_ref(&hidden))
        .unwrap();
    let r = rows(&store);
    assert_eq!(
        short(&r),
        [
            (true, "09:00".into(), 60, "Loyalty".into()),
            (true, "10:00".into(), 30, "UX gösterimi".into())
        ]
    );
    assert_eq!(r[1].entry.hours, 0.5);
}

#[test]
fn a_row_moved_onto_saved_work_does_not_write_it_twice() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "kum", "Kum");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg", "kum"]);
    let (r, _) = rows(&store, &togg);
    store.save_timesheet_entry(None, &r[0].entry).unwrap();
    // İşin yarısı raporda Kum'a alındı, Kum satırı kaydedildi; sonra Togg'a geri taşındı.
    store
        .set_project_between(t(30), t(60), Some("kum"))
        .unwrap();
    let (r, _) = rows(&store, &togg);
    let kum = r.iter().find(|r| r.entry.project_id == "kum").unwrap();
    let id = store.save_timesheet_entry(None, &kum.entry).unwrap();
    store
        .set_project_between(t(30), t(60), Some("togg"))
        .unwrap();
    store
        .save_timesheet_entry(
            Some(&id),
            &TimesheetEntry {
                project_id: "togg".into(),
                ..kum.entry.clone()
            },
        )
        .unwrap();
    // Togg'un satırı zaten o yarım saati kapsıyor: taşınan satır kalmaz.
    let (r, _) = rows(&store, &togg);
    assert_eq!(short(&r), [(true, "09:00".into(), 60, "Loyalty".into())]);
    assert_eq!(r[0].stale, None);
}

#[test]
fn deleting_the_rest_of_a_longer_meeting_keeps_the_saved_part() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    let togg = sheet("togg", &["togg"]);
    let meeting = |to: i64| Meeting {
        uid: "plan".into(),
        start: t(0),
        end: t(to),
        subject: "Haftalık plan".into(),
        online: true,
        ..Meeting::default()
    };
    store.assign_meeting("plan", Some("togg")).unwrap();
    let day_rows = |store: &Store, m: &Meeting| {
        let ctx = store.timesheet_context().unwrap();
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), std::slice::from_ref(m))
            .unwrap();
        store
            .timesheet_day(&ctx, &togg, day(), &pieces)
            .unwrap()
            .rows
    };
    // Bir saatlik toplantı kaydedildi.
    let short = meeting(60);
    let r = day_rows(&store, &short);
    let saved = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    // Toplantı uzadı: artığı ayrı satır olarak gelir ve silinir.
    let long = meeting(90);
    let r = day_rows(&store, &long);
    let rest = r.iter().find(|r| r.id.is_none()).expect("artık önerilir");
    store.dismiss_timesheet_entry(None, &rest.entry).unwrap();
    // Kaydedilen saat yerinde kalır, eskimez; toplantı "yapılmadı" sayılmaz.
    let r = day_rows(&store, &long);
    let kept: Vec<_> = r.iter().filter_map(|r| r.id.clone()).collect();
    assert_eq!(kept, [saved]);
    assert!(r.iter().all(|r| r.stale.is_none()), "{r:?}");
    assert!(
        r.iter().all(|r| r.id.is_some()),
        "silinen artık geri gelmez: {r:?}"
    );
}

#[test]
fn merging_live_and_saved_rows_and_undoing_it() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 20, Some("togg"));
    work(&store, "Rapor", 60, 70, Some("togg"));
    work(&store, "Loyalty", 120, 140, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    assert_eq!(r.len(), 3);
    // Ortadaki düzenlendi (kaydedildi), diğer ikisi canlı.
    let saved = TimesheetEntry {
        details: "Haftalık rapor".into(),
        ..r[1].entry.clone()
    };
    let mid = store.save_timesheet_entry(None, &saved).unwrap();
    let input: Vec<(Option<String>, TimesheetEntry)> = vec![
        (None, r[0].entry.clone()),
        (Some(mid.clone()), r[1].entry.clone()),
        (None, r[2].entry.clone()),
    ];
    let (merged, removed) = store.merge_timesheet_entries(&input).unwrap();
    let (r2, _) = rows(&store, &togg);
    assert_eq!(
        short(&r2),
        [(true, "09:00".into(), 50, "Loyalty; Haftalık rapor".into())]
    );
    assert_eq!(r2[0].entry.hours, 0.75);
    assert_eq!(r2[0].key, r[0].key);
    assert_eq!(removed, vec![(mid.clone(), saved.clone())]);
    // Geri al: kaydedilmiş satır aynen, canlılar canlı.
    store.unmerge_timesheet_entries(&merged, &removed).unwrap();
    let (r3, _) = rows(&store, &togg);
    assert_eq!(
        short(&r3),
        [
            (false, "09:00".into(), 20, "Loyalty".into()),
            (true, "10:00".into(), 10, "Haftalık rapor".into()),
            (false, "11:00".into(), 20, "Loyalty".into())
        ]
    );
    assert!(store.unmerge_timesheet_entries(&merged, &removed).is_err());
}

#[test]
fn work_moved_to_another_project_marks_the_row_stale() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "kum", "Kum");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store
        .save_timesheet_entry(
            None,
            &TimesheetEntry {
                details: "Loyalty ekranları".into(),
                ..r[0].entry.clone()
            },
        )
        .unwrap();
    assert_eq!(rows(&store, &togg).0[0].stale, None);
    // İlk yarım saat raporda Kum'a alındı.
    store.set_project_between(t(0), t(30), Some("kum")).unwrap();
    let (r, _) = rows(&store, &togg);
    assert_eq!(r.len(), 1, "Kum'un işi Togg'a önerilmez");
    assert_eq!(r[0].stale, Some(0.5));
    let ctx = store.timesheet_context().unwrap();
    let pieces = store.timesheet_pieces(&ctx, t(-540), t(900), &[]).unwrap();
    assert!(store.refresh_timesheet_entry(&id, &pieces).unwrap());
    let (r, _) = rows(&store, &togg);
    assert_eq!(
        short(&r),
        [(true, "09:30".into(), 30, "Loyalty ekranları".into())]
    );
    assert_eq!(r[0].stale, None);
    // Kalanı da gidince güncelleme satırı siler.
    store
        .set_project_between(t(30), t(60), Some("kum"))
        .unwrap();
    let pieces = store.timesheet_pieces(&ctx, t(-540), t(900), &[]).unwrap();
    assert!(!store.refresh_timesheet_entry(&id, &pieces).unwrap());
    assert!(rows(&store, &togg).0.is_empty());
}

#[test]
fn exported_rows_stay_with_their_sheet_and_cannot_change() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    store
        .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), &togg.id, "")
        .unwrap();
    assert!(store.save_timesheet_entry(Some(&id), &r[0].entry).is_err());
    assert!(
        store
            .dismiss_timesheet_entry(Some(&id), &r[0].entry)
            .is_err()
    );
    assert_eq!(store.reset_timesheet_day(&togg, day()).unwrap(), 0);
    // Proje başka çizelgeye taşınsa da aktarılan satır gittiği çizelgede görünür.
    let (r, _) = rows(&store, &togg);
    assert!(r[0].exported);
    assert!(rows(&store, &sheet("yeni", &["togg"])).0.is_empty());
    // Aktarılan iş yeniden önerilmez; birleştirilemez.
    assert_eq!(r.len(), 1);
    assert!(
        store
            .merge_timesheet_entries(&vec![(Some(id.clone()), r[0].entry.clone()); 2])
            .is_err()
    );
    // Aktarım geri alınınca satır yeniden düzenlenebilir.
    store
        .unmark_timesheet_exported(std::slice::from_ref(&id))
        .unwrap();
    assert!(store.save_timesheet_entry(Some(&id), &r[0].entry).is_ok());
}

#[test]
fn exported_rows_follow_their_file_row() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    // Aktarılmamış satır bu yoldan değişmez.
    assert!(
        store
            .save_exported_entry(&id, &r[0].entry, false, None)
            .is_err()
    );
    store
        .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), &togg.id, "")
        .unwrap();
    let edited = TimesheetEntry {
        details: "Loyalty ekranları".into(),
        hours: 1.25,
        coverage: Some(Vec::new()),
        ..r[0].entry.clone()
    };
    store
        .save_exported_entry(&id, &edited, false, None)
        .unwrap();
    let saved = store.timesheet_entry(&id).unwrap().unwrap();
    assert!(saved.exported_at.is_some());
    assert_eq!(
        (saved.entry.details.as_str(), saved.entry.hours),
        ("Loyalty ekranları", 1.25)
    );
    assert_eq!(
        saved.entry.coverage, r[0].entry.coverage,
        "aralıklar istenmedikçe değişmez"
    );
    assert!(
        store
            .save_exported_entry(
                &id,
                &TimesheetEntry {
                    hours: 0.0,
                    ..edited.clone()
                },
                false,
                None
            )
            .is_err()
    );

    // Aktarılmış satır da takipte değişir (iş raporda projesize alındı).
    store
        .set_project_between(t(0), t(30), Some(crate::classify::NO_PROJECT))
        .unwrap();
    let (r, _) = rows(&store, &togg);
    assert_eq!(r[0].stale, Some(0.5));

    // Dosyadan kaldırılınca gizlenir; geri getirilince yeniden gönderilebilir.
    store.withdraw_exported_entry(&id, true).unwrap();
    let (r, hidden) = rows(&store, &togg);
    assert_eq!(hidden, 1);
    assert!(r.is_empty(), "aralıkları yeniden önerilmez: {r:?}");
    store.restore_hidden(&togg, day()).unwrap();
    let (r, _) = rows(&store, &togg);
    assert_eq!(
        (r[0].id.as_deref(), r[0].exported),
        (Some(id.as_str()), false)
    );
    store
        .mark_timesheet_exported(std::slice::from_ref(&id), Utc::now(), &togg.id, "")
        .unwrap();
    store.withdraw_exported_entry(&id, false).unwrap();
    assert!(store.timesheet_entry(&id).unwrap().is_none());
}

#[test]
fn manual_rows_and_reset() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let manual = TimesheetEntry {
        date: day(),
        start: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
        hours: 1.0,
        actual_hours: None,
        kind: EntryKind::F2F,
        details: "Atölye".into(),
        party: "ADBA".into(),
        project_id: "togg".into(),
        division: "Togg".into(),
        coverage: None,
    };
    let id = store.save_timesheet_entry(None, &manual).unwrap();
    // Elle eklenen satır aralıksızdır: takipteki işi düşmez.
    assert_eq!(
        store.timesheet_entry(&id).unwrap().unwrap().entry.coverage,
        Some(Vec::new())
    );
    let (r, _) = rows(&store, &togg);
    assert_eq!(r.len(), 2);
    let (live, _) = rows(&store, &togg);
    store
        .save_timesheet_entry(
            None,
            &TimesheetEntry {
                details: "x".into(),
                ..live[0].entry.clone()
            },
        )
        .unwrap();
    // Yeniden öner: düzenlemeler ve elle eklenenler gider, iş yeniden öneri olur.
    assert_eq!(store.reset_timesheet_day(&togg, day()).unwrap(), 2);
    let (r, _) = rows(&store, &togg);
    assert_eq!(short(&r), [(false, "09:00".into(), 60, "Loyalty".into())]);
    assert!(
        store
            .save_timesheet_entry(
                None,
                &TimesheetEntry {
                    hours: 0.0,
                    ..manual
                }
            )
            .is_err()
    );
}

#[test]
fn legacy_rows_are_still_subtracted_by_hours() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Rapor", 120, 150, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    // Önceki sürümde onaylanıp aktarılmış bir saatlik satır (aralığı yok).
    let (r, _) = rows(&store, &togg);
    let old = TimesheetEntry {
        coverage: None,
        ..r[0].entry.clone()
    };
    store.insert_entry("eski", &old).unwrap();
    store
        .mark_timesheet_exported(&["eski".into()], Utc::now(), &togg.id, "")
        .unwrap();
    let (r, _) = rows(&store, &togg);
    assert_eq!(
        short(&r),
        [
            (true, "09:00".into(), 60, "Loyalty".into()),
            (false, "11:00".into(), 30, "Rapor".into())
        ]
    );
}

#[test]
fn single_timesheet_moves_to_its_company() {
    let store = Store::open_in_memory().unwrap();
    // Şablondan gelen birim projeleri silinmiş; iş "Togg" projesinde, birim satırda seçiliyor.
    project(&store, "togg", "Togg");
    project(&store, "kum", "tracky");
    project(&store, "sync", "Int.Work.Sync.");
    store.delete_tag("sync").unwrap();
    let adba = Client {
        id: "adba".into(),
        name: "ADBA".into(),
    };
    store.upsert_client(&adba, 0).unwrap();
    store.set_project_client("togg", Some("adba")).unwrap();
    store
        .save_setting(
            TIMESHEET_KEY,
            &serde_json::json!({
                "company": "Togg",
                "consultant": "Kaan",
                "filePath": "/tmp/sablon.xlsx",
                "sheetUrl": "https://script.google.com/macros/s/x/exec",
                "sheetLink": null,
                "sheetToken": "anahtar",
                "defaultParty": "ADBA",
                "projects": [
                    {"projectId": "sync", "division": "Int.Work.Sync.", "party": null},
                    {"projectId": "yok", "division": "Trumore", "party": null}
                ],
                "meetingApps": ["us.zoom.xos"],
                "dayHours": 7.5
            }),
        )
        .unwrap();
    let row = |project: &str, division: &str| TimesheetEntry {
        date: day(),
        start: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        hours: 1.0,
        actual_hours: Some(1.0),
        kind: EntryKind::Online,
        details: "Toplantı".into(),
        party: "ADBA".into(),
        project_id: project.into(),
        division: division.into(),
        coverage: None,
    };
    store
        .insert_entry("a", &row("sync", "Int.Work.Sync."))
        .unwrap();
    store.insert_entry("b", &row("kum-eski", "tracky")).unwrap();
    store.insert_entry("c", &row("togg", "Togg")).unwrap();
    store
        .mark_timesheet_exported(&["a".into(), "b".into()], Utc::now(), "", "")
        .unwrap();
    store
        .conn
        .execute("UPDATE timesheet_entries SET timesheet_id = NULL", [])
        .unwrap();

    store.migrate_timesheets().unwrap();
    let config = store.timesheet_config().unwrap();
    assert_eq!(
        (
            config.sheet_token.as_str(),
            config.day_hours,
            config.meeting_apps.len()
        ),
        ("anahtar", 7.5, 1)
    );
    let [togg] = &config.timesheets[..] else {
        panic!("tek çizelge: {config:?}");
    };
    assert_eq!(togg.id, first_timesheet_id());
    assert_eq!(
        (
            togg.company.as_str(),
            togg.consultant.as_str(),
            togg.sheet_url.is_some()
        ),
        ("Togg", "Kaan", true)
    );
    // Yalnızca Togg projesi; kişisel proje (tracky) girmez.
    let projects: Vec<&str> = togg
        .projects
        .iter()
        .map(|m| m.project_id.as_str())
        .collect();
    assert_eq!(projects, ["togg"]);
    assert_eq!(togg.divisions, ["Int.Work.Sync.", "Trumore"]);
    let saved = store.timesheet_entries(day(), day()).unwrap();
    let by_id = |id: &str| saved.iter().find(|s| s.id == id).unwrap();
    // Silinmiş birim projesine dönmüş satır Togg'a bağlanır; birimi korunur.
    assert_eq!(
        (
            by_id("a").entry.project_id.as_str(),
            by_id("a").entry.division.as_str()
        ),
        ("togg", "Int.Work.Sync.")
    );
    assert_eq!(by_id("a").timesheet_id.as_deref(), Some(togg.id.as_str()));
    // Togg'a gitmiş başka projenin satırı orada görünmeye devam eder, projesi değişmez.
    assert_eq!(by_id("b").entry.project_id, "kum-eski");
    assert_eq!(by_id("b").timesheet_id.as_deref(), Some(togg.id.as_str()));
    assert_eq!(by_id("c").timesheet_id, None);

    // Bir kez çalışır.
    store.migrate_timesheets().unwrap();
    assert_eq!(store.timesheet_config().unwrap(), config);
}

#[test]
fn nothing_to_move_without_a_timesheet() {
    let store = Store::open_in_memory().unwrap();
    store.migrate_timesheets().unwrap();
    assert!(
        store
            .setting::<serde_json::Value>(TIMESHEET_KEY)
            .unwrap()
            .is_none()
    );
    store
        .save_setting(TIMESHEET_KEY, &serde_json::json!({ "dayHours": 6 }))
        .unwrap();
    store.migrate_timesheets().unwrap();
    let config = store.timesheet_config().unwrap();
    assert!(config.timesheets.is_empty());
    assert_eq!(config.day_hours, 6.0);
    assert!(!config.meeting_apps.is_empty());
}

#[test]
fn meeting_suggester_learns_from_assigned_series_and_skips_archived() {
    let store = Store::open_in_memory().unwrap();
    let tag = |id: &str, name: &str| crate::classify::Tag {
        id: id.into(),
        kind: TagKind::Project,
        name: name.into(),
        color: 1,
    };
    store.upsert_tag(&tag("p1", "Portal"), 0).unwrap();
    let series = |uid: &str, subject: &str| Meeting {
        uid: uid.into(),
        start: chrono::Utc::now(),
        end: chrono::Utc::now() + chrono::Duration::hours(1),
        subject: subject.into(),
        attendees: vec!["me@kum.dev".into(), "ali@acme.com".into()],
        ..Meeting::default()
    };
    // İç toplantı: kullanıcının kendi alan adı en sık geçen olur.
    let mut internal = series("c", "Ekip");
    internal.attendees.truncate(1);
    let all = [
        series("a", "Planlama"),
        series("b", "Retro"),
        internal,
        series("new", "Yeni konu"),
    ];
    store.assign_meeting("a", Some("p1")).unwrap();
    store.assign_meeting("b", Some("p1")).unwrap();
    let got = store
        .meeting_suggester(&all)
        .unwrap()
        .suggest(&all[3])
        .unwrap();
    assert_eq!(got.project_id, "p1");
    assert_eq!(got.reason, "katılımcılar @acme.com");
    // Arşivlenen proje önerilmez.
    store.archive_project("p1").unwrap();
    assert_eq!(
        store.meeting_suggester(&all).unwrap().suggest(&all[3]),
        None
    );
}

#[test]
fn meetings_follow_calls_and_skipped_ones_leave_their_time() {
    use crate::calls::Call;
    use crate::store::DEVICE_KEY_PREFIX;
    use crate::store::devices::DeviceInfo;

    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "trumore", "Trumore");
    let togg = sheet("togg", &["togg", "trumore"]);
    let meeting = |uid: &str, subject: &str, from: i64, to: i64| Meeting {
        uid: uid.into(),
        start: t(from),
        end: t(to),
        subject: subject.into(),
        online: true,
        ..Meeting::default()
    };
    let meetings = [
        meeting("plan", "Haftalık plan", 0, 60),
        meeting("ux", "Charging UX", 120, 180),
    ];
    store.assign_meeting("plan", Some("togg")).unwrap();
    store.assign_meeting("ux", Some("togg")).unwrap();
    // Planlama süresince görüşme yok, başka projede çalışıldı; UX görüşmesi yarım saat sürdü.
    work(&store, "Trumore deck", -10, 60, Some("trumore"));
    store
        .upsert_call(&Call {
            id: Uuid::new_v4(),
            app_id: "com.microsoft.teams2".into(),
            started_at: t(119),
            ended_at: t(150),
        })
        .unwrap();
    let rows = |store: &Store| {
        let ctx = store.timesheet_context().unwrap();
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
            .unwrap();
        let d = store.timesheet_day(&ctx, &togg, day(), &pieces).unwrap();
        d.rows
            .into_iter()
            .map(|r| ((r.entry.worked() * 60.0).round() as i64, r.entry.details))
            .collect::<Vec<_>>()
    };
    // Bu bilgisayar görüşmeleri kaydetmiyordu: planlama davetteki gibi.
    store.register_device("Mac", "macos", "", "").unwrap();
    assert_eq!(
        rows(&store),
        [
            (10, "Trumore deck".to_string()),
            (60, "Haftalık plan".to_string()),
            (30, "Charging UX".to_string()),
        ]
    );
    // Kaydediyordu: planlamaya katılınmadı, süre Trumore'a kalır.
    let key = format!("{DEVICE_KEY_PREFIX}{}", store.device_id());
    let mut info: DeviceInfo = store.setting(&key).unwrap().unwrap();
    info.calls_from = Some(t(-600));
    store.save_setting(&key, &info).unwrap();
    assert_eq!(
        rows(&store),
        [
            (70, "Trumore deck".to_string()),
            (30, "Charging UX".to_string()),
        ]
    );
    // "Katıldım" denince geri gelir.
    let plan = crate::attendance::key(&meetings[0]);
    store.answer_meeting(&plan, Some(true)).unwrap();
    assert_eq!(rows(&store)[1], (60, "Haftalık plan".to_string()));
    store.answer_meeting(&plan, None).unwrap();
    assert_eq!(rows(&store).len(), 2);
}

#[test]
fn work_saved_on_two_computers_is_marked_until_refreshed() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 90, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let (r, _) = rows(&store, &togg);
    let id = store.save_timesheet_entry(None, &r[0].entry).unwrap();
    // Öteki bilgisayarda eşitlenmeden kaydedilen satır: aynı işin son yarım saati.
    let other = TimesheetEntry {
        start: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        hours: 0.5,
        actual_hours: Some(0.5),
        coverage: Some(timesheet::to_coverage(&[(t(60), t(90))])),
        ..r[0].entry.clone()
    };
    store.insert_entry("zz-other", &other).unwrap();
    // Sonra gelen satırda tekrarlanan süre takipte değişmiş sayılır (gönderilmez).
    let (r, _) = rows(&store, &togg);
    let stale: Vec<_> = r.iter().map(|r| (r.id.clone().unwrap(), r.stale)).collect();
    assert_eq!(stale, [(id.clone(), None), ("zz-other".into(), Some(0.0))]);
    // Güncellenince o süre düşülür: işi kalmayan satır silinir.
    let ctx = store.timesheet_context().unwrap();
    let pieces = store.timesheet_pieces(&ctx, t(-540), t(900), &[]).unwrap();
    assert!(!store.refresh_timesheet_entry("zz-other", &pieces).unwrap());
    let (r, _) = rows(&store, &togg);
    assert_eq!(short(&r), [(true, "09:00".into(), 90, "Loyalty".into())]);
    assert_eq!(r[0].stale, None);
}

/// Bloklar `min` dakikada seçerek gönderilmeye başlandı: ondan önce başlayan bloklar gönderilmiş
/// sayılır.
fn opt_in_at(store: &Store, min: i64) {
    store.set_selective_since(t(min)).unwrap();
}

fn send(store: &Store, from: i64, to: i64, project: Option<&str>) {
    store
        .send_block((t(from), t(to)), project, &[t(-540), t(900)], &[])
        .unwrap();
}

fn remove(store: &Store, from: i64, to: i64, project: Option<&str>) -> Unsent {
    store
        .remove_block((t(from), t(to)), project, &[t(-540), t(900)], &[])
        .unwrap()
}

#[test]
fn only_sent_blocks_become_rows_and_rows_follow_their_block() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    opt_in_at(&store, -60);
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Rapor", 180, 240, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    // Gönderilmeyen blok satır olmaz.
    assert!(rows(&store, &togg).0.is_empty());

    // Gönderilen blok satır olur; öteki gönderilmedikçe gelmez.
    send(&store, 0, 60, Some("togg"));
    assert_eq!(
        short(&rows(&store, &togg).0),
        [(false, "09:00".into(), 60, "Loyalty".into())]
    );

    // Blok uzayınca (ya da içine iş atanınca) satırı da uzar.
    work(&store, "Loyalty", 60, 90, Some("togg"));
    assert_eq!(
        short(&rows(&store, &togg).0),
        [(false, "09:00".into(), 90, "Loyalty".into())]
    );

    // Kısaltılan bloğun kesilen kısmı ayrı blok olur ve gönderilmemiş sayılır.
    store
        .resize_block(t(0), t(90), t(0), t(45), "Togg", None, Some("togg"))
        .unwrap();
    assert_eq!(
        short(&rows(&store, &togg).0),
        [(false, "09:00".into(), 45, "Loyalty".into())]
    );

    // Çizelgeden çıkan bloğun satırı gider.
    assert_eq!(remove(&store, 0, 45, Some("togg")), Unsent::default());
    assert!(rows(&store, &togg).0.is_empty());
}

#[test]
fn blocks_before_opting_in_stay_in_the_timesheet() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Rapor", 180, 240, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    opt_in_at(&store, 120);
    assert_eq!(
        short(&rows(&store, &togg).0),
        [(false, "09:00".into(), 60, "Loyalty".into())]
    );
    // Bilgisayarlar farklı zamanlarda güncellenir: en erken geçen geçerli.
    store
        .conn
        .execute(
            "INSERT INTO timesheet_marks (id, kind, started_at, ended_at, updated_at)
             VALUES ('since:other', 'since', ?1, ?1, ?1)",
            [ms(t(-60))],
        )
        .unwrap();
    assert!(rows(&store, &togg).0.is_empty());
}

#[test]
fn meetings_enter_without_sending() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    opt_in_at(&store, -60);
    work(&store, "Loyalty", 0, 90, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    let meetings = [Meeting {
        uid: "plan".into(),
        start: t(60),
        end: t(90),
        subject: "Haftalık plan".into(),
        online: true,
        ..Meeting::default()
    }];
    store.assign_meeting("plan", Some("togg")).unwrap();
    let ctx = store.timesheet_context().unwrap();
    let day_rows = || {
        let pieces = store
            .timesheet_pieces(&ctx, t(-540), t(900), &meetings)
            .unwrap();
        short(
            &store
                .timesheet_day(&ctx, &togg, day(), &pieces)
                .unwrap()
                .rows,
        )
    };
    assert_eq!(
        day_rows(),
        [(false, "10:00".into(), 30, "Haftalık plan".into())]
    );
    send(&store, 0, 90, Some("togg"));
    assert_eq!(
        day_rows(),
        [
            (false, "09:00".into(), 60, "Loyalty".into()),
            (false, "10:00".into(), 30, "Haftalık plan".into())
        ]
    );
}

#[test]
fn sending_again_brings_back_a_deleted_row() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    opt_in_at(&store, -60);
    work(&store, "Loyalty", 0, 60, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    send(&store, 0, 60, Some("togg"));
    let (r, _) = rows(&store, &togg);
    store.dismiss_timesheet_entry(None, &r[0].entry).unwrap();
    assert_eq!(rows(&store, &togg).1, 1);
    remove(&store, 0, 60, Some("togg"));
    send(&store, 0, 60, Some("togg"));
    assert_eq!(
        short(&rows(&store, &togg).0),
        [(false, "09:00".into(), 60, "Loyalty".into())]
    );
}

#[test]
fn report_blocks_tell_whether_they_were_sent() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    project(&store, "kum", "Kum");
    opt_in_at(&store, -60);
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Kum", 120, 180, Some("kum"));
    work(&store, "Rapor", 240, 300, Some("togg"));
    store
        .save_timesheet_config(&TimesheetConfig {
            timesheets: vec![sheet("togg", &["togg"])],
            ..Default::default()
        })
        .unwrap();
    send(&store, 0, 60, Some("togg"));
    let report = store.report(t(-540), t(900), &[t(-540)], true).unwrap();
    let sent: Vec<_> = report
        .work
        .blocks
        .iter()
        .map(|b| (b.project_id.as_deref(), b.sent))
        .collect();
    // Çizelgeye bağlı olmayan projenin bloğunda bilgi yok.
    assert_eq!(
        sent,
        [
            (Some("togg"), Some(true)),
            (Some("kum"), None),
            (Some("togg"), Some(false))
        ]
    );
}

#[test]
fn removing_a_block_hides_its_rows_and_undo_brings_them_back() {
    let store = Store::open_in_memory().unwrap();
    project(&store, "togg", "Togg");
    work(&store, "Loyalty", 0, 60, Some("togg"));
    work(&store, "Rapor", 90, 150, Some("togg"));
    let togg = sheet("togg", &["togg"]);
    store
        .save_timesheet_config(&TimesheetConfig {
            timesheets: vec![togg.clone()],
            ..Default::default()
        })
        .unwrap();
    // Seçerek göndermeden önceki blok: satırı canlı. Düzenlenen satır da kaydedilir.
    opt_in_at(&store, 180);
    let (r, _) = rows(&store, &togg);
    assert_eq!(r.len(), 2);
    store
        .save_timesheet_entry(
            None,
            &TimesheetEntry {
                details: "Rapor yazımı".into(),
                ..r[1].entry.clone()
            },
        )
        .unwrap();
    // Çıkarılan blokların satırları gider: canlı satır gönderilmemiş sayıldığı için, düzenlenen
    // satır gizlenerek.
    let gone = remove(&store, 0, 150, Some("togg"));
    assert_eq!(gone.hidden.len(), 1);
    assert_eq!(rows(&store, &togg), (Vec::new(), 1));
    let report = store.report(t(-540), t(900), &[t(-540)], true).unwrap();
    assert!(report.work.blocks.iter().all(|b| b.sent == Some(false)));
    // Geri alınca aynen geri gelir.
    store.restore_block((t(0), t(150)), &gone.hidden).unwrap();
    assert_eq!(
        short(&rows(&store, &togg).0),
        [
            (false, "09:00".into(), 60, "Loyalty".into()),
            (true, "10:30".into(), 60, "Rapor yazımı".into())
        ]
    );
}
