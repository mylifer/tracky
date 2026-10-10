use super::*;
use crate::classify::{NO_PROJECT, Rule, RuleField, Tag, TagKind};
use uuid::Uuid;

fn t(min: i64) -> DateTime<Utc> {
    Local
        .with_ymd_and_hms(2026, 10, 1, 9, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
        + Duration::minutes(min)
}

fn s(app: &str, title: &str, from: i64, to: i64, project: Option<&str>) -> Session {
    Session {
        id: Uuid::new_v4(),
        app_id: app.into(),
        app_name: app.into(),
        title: title.into(),
        url: None,
        domain: None,
        started_at: t(from),
        ended_at: t(to),
        category_id: None,
        project_id: project.map(Into::into),
        block_from: None,
    }
}

fn mapping(project: &str, division: &str) -> ProjectMapping {
    ProjectMapping {
        project_id: project.into(),
        division: division.into(),
        party: None,
        default_details: None,
    }
}

fn setup() -> (
    Classifier,
    HashMap<String, String>,
    TimesheetConfig,
    Timesheet,
) {
    let tag = |id: &str, name: &str| Tag {
        id: id.into(),
        kind: TagKind::Project,
        name: name.into(),
        color: 1,
    };
    let classifier = Classifier::new(
        &[
            tag("tru", "Trumore"),
            tag("sync", "Int.Work.Sync."),
            tag("kum", "Kum"),
        ],
        &[
            Rule {
                id: "r".into(),
                tag_id: "tru".into(),
                field: RuleField::Title,
                pattern: "trumore".into(),
            },
            Rule {
                id: "k".into(),
                tag_id: "kum".into(),
                field: RuleField::Title,
                pattern: "kum".into(),
            },
        ],
    );
    let names = HashMap::from([
        ("tru".to_string(), "Trumore".to_string()),
        ("sync".to_string(), "Int.Work.Sync.".to_string()),
        ("kum".to_string(), "Kum".to_string()),
    ]);
    let sheet = Timesheet {
        id: "togg".into(),
        company: "Togg".into(),
        default_party: "ADBA".into(),
        // Kum projesi bu çizelgeye bağlı değil: önerilmez.
        projects: vec![mapping("tru", ""), mapping("sync", "Int.Work.Sync.")],
        ..Default::default()
    };
    (classifier, names, TimesheetConfig::default(), sheet)
}

/// Tek günün (kaydedilmiş satırı olmayan) önerileri.
fn day(
    sessions: &[Session],
    meetings: &[(Meeting, String)],
    classifier: &Classifier,
    names: &HashMap<String, String>,
    config: &TimesheetConfig,
    sheet: &Timesheet,
) -> Vec<TimesheetEntry> {
    let p = pieces(sessions, meetings, classifier, config, t(-540), t(900));
    propose(&p, names, sheet, &HashMap::new())
}

#[test]
fn a_calendar_block_is_one_row() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore Loyalty UI/UX — Figma", 0, 50, None),
        s("Slack", "#genel", 50, 55, None), // atanmamış: bloğun projesine girer
        s("Figma", "Trumore Loyalty UI/UX — Figma", 55, 80, None),
        s("Slack", "#kişisel", 80, 90, Some(NO_PROJECT)), // projesiz: girmez
        s("us.zoom.xos", "Zoom Meeting", 90, 120, Some("tru")), // görüşme: aynı satır
        s("Figma", "Trumore Pitchdeck — Figma", 150, 170, None), // 30 dk boşluk: yeni blok
        s("Slack", "sync", 170, 173, Some("sync")),       // 3 dk: çok kısa
        s("Code", "kum — main.rs", 180, 240, None),       // başka çizelgenin projesi
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let rows: Vec<_> = got
        .iter()
        .map(|e| {
            (
                e.start.format("%H:%M").to_string(),
                (e.worked() * 60.0).round() as i64,
                e.kind,
                e.details.as_str(),
                e.division.as_str(),
                e.party.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "09:00".to_string(),
                110,
                EntryKind::Working,
                "Trumore Loyalty UI/UX",
                "Trumore",
                "ADBA"
            ),
            (
                "11:30".to_string(),
                20,
                EntryKind::Working,
                "Trumore Pitchdeck",
                "Trumore",
                "ADBA"
            ),
        ]
    );
    // Kaydın aralıkları projesiz süre olmadan saklanır.
    assert_eq!(
        got[0].spans(),
        vec![(t(0), t(80)), (t(90), t(120))],
        "aradaki projesiz 10 dk kayda girmez"
    );
}

#[test]
fn a_split_block_is_two_rows() {
    let (classifier, names, config, sheet) = setup();
    let mut after = s("Figma", "Trumore Rapor — Figma", 60, 120, None);
    after.block_from = Some(t(60));
    let sessions = [
        s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
        after,
        s("Slack", "#genel", 120, 125, None), // atanmamış: bölünen ikinci bloğa girer
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let rows: Vec<_> = got
        .iter()
        .map(|e| {
            (
                e.start.format("%H:%M").to_string(),
                (e.worked() * 60.0).round() as i64,
            )
        })
        .collect();
    assert_eq!(rows, [("09:00".to_string(), 60), ("10:00".to_string(), 65)]);
}

#[test]
fn a_row_closed_by_a_gap_takes_its_longest_kind() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("us.zoom.xos", "Zoom Meeting", 0, 10, Some("tru")),
        s("Figma", "Trumore Loyalty — Figma", 10, 120, None),
        s("Figma", "Trumore Rapor — Figma", 240, 300, None), // uzun boşluk: yeni satır
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let kinds: Vec<_> = got.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [EntryKind::Working, EntryKind::Working]);
}

#[test]
fn only_the_sheets_projects_are_proposed() {
    let (classifier, names, config, mut sheet) = setup();
    let sessions = [
        s("Figma", "Trumore — Figma", 0, 60, None),
        s("Code", "kum — main.rs", 60, 120, None),
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].project_id, "tru");
    // Kişisel projenin çizelgesi: yalnızca Kum.
    sheet.projects = vec![mapping("kum", "")];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].project_id.as_str(), got[0].division.as_str()),
        ("kum", "Kum")
    );
    // Projesi olmayan çizelge hiçbir şey almaz.
    sheet.projects.clear();
    assert!(day(&sessions, &[], &classifier, &names, &config, &sheet).is_empty());
}

#[test]
fn saved_spans_are_not_proposed_again() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
        s("Figma", "Trumore Rapor — Figma", 120, 180, None),
    ];
    let p = pieces(&sessions, &[], &classifier, &config, t(-540), t(900));
    let all = propose(&p, &names, &sheet, &HashMap::new());
    assert_eq!(all.len(), 2);
    // İkinci satır kaydedildi (düzenlendi): ilki önerilmeye devam eder, ikincisi gelmez.
    let saved = HashMap::from([("tru".to_string(), vec![all[1].spans()])]);
    let left = propose(&p, &names, &sheet, &saved);
    assert_eq!(left, vec![all[0].clone()]);

    // Sabah unutulan iş sonradan projeye atanınca kendi saatinde gelir.
    let mut later = sessions.to_vec();
    later.push(s("Mail", "Rapor taslağı", -120, -60, Some("tru")));
    let p = pieces(&later, &[], &classifier, &config, t(-540), t(900));
    let saved = HashMap::from([("tru".to_string(), vec![all[0].spans(), all[1].spans()])]);
    let got = propose(&p, &names, &sheet, &saved);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].start.format("%H:%M").to_string(), "07:00");
    assert_eq!(got[0].details, "Rapor taslağı");

    // Satır kaydedilirken süren işin kısa kuyruğu önerilmez; uzayınca önerilir.
    let mut tail = sessions.to_vec();
    tail.push(s("Figma", "Trumore Rapor — Figma", 180, 190, None));
    let p = pieces(&tail, &[], &classifier, &config, t(-540), t(900));
    let saved = HashMap::from([("tru".to_string(), vec![all[0].spans(), all[1].spans()])]);
    assert!(propose(&p, &names, &sheet, &saved).is_empty());
    tail.push(s("Figma", "Trumore Rapor — Figma", 190, 200, None));
    let p = pieces(&tail, &[], &classifier, &config, t(-540), t(900));
    let got = propose(&p, &names, &sheet, &saved);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].start.format("%H:%M").to_string(), "12:00");
    assert_eq!(got[0].spans(), vec![(t(180), t(200))]);
}

#[test]
fn work_assigned_later_inside_a_saved_row_is_proposed() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore Loyalty — Figma", 0, 20, None),
        s("Slack", "#genel", 20, 30, Some(NO_PROJECT)),
        s("Figma", "Trumore Loyalty — Figma", 30, 60, None),
    ];
    let p = pieces(&sessions, &[], &classifier, &config, t(-540), t(900));
    let row = propose(&p, &names, &sheet, &HashMap::new()).remove(0);
    assert_eq!(row.spans(), vec![(t(0), t(20)), (t(30), t(60))]);
    // Satır kaydedildi (ya da silindi); aradaki 6 dakika sonradan raporda projeye atandı:
    // satırın artığı değil, yeni iş.
    let saved = HashMap::from([("tru".to_string(), vec![row.spans()])]);
    let mut later = sessions.to_vec();
    later[1] = s("Slack", "#genel", 20, 26, Some("tru"));
    let p = pieces(&later, &[], &classifier, &config, t(-540), t(900));
    let got = propose(&p, &names, &sheet, &saved);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].spans(), vec![(t(20), t(26))]);
    // Satırın dışındaki kısa kuyruk ise önerilmez.
    let mut tail = sessions.to_vec();
    tail.push(s("Figma", "Trumore Loyalty — Figma", 60, 70, None));
    let p = pieces(&tail, &[], &classifier, &config, t(-540), t(900));
    assert!(propose(&p, &names, &sheet, &saved).is_empty());
}

#[test]
fn a_short_meeting_after_a_saved_one_is_still_proposed() {
    let (classifier, names, config, sheet) = setup();
    let first = (
        meeting("a", "Trumore haftalık", 0, 60, true),
        "tru".to_string(),
    );
    let second = (
        meeting("b", "Trumore kısa", 60, 70, true),
        "tru".to_string(),
    );
    let p = pieces(&[], &[first, second], &classifier, &config, t(-540), t(900));
    let all = propose(&p, &names, &sheet, &HashMap::new());
    assert_eq!(all.len(), 2);
    let saved = HashMap::from([("tru".to_string(), vec![all[0].spans()])]);
    assert_eq!(propose(&p, &names, &sheet, &saved), vec![all[1].clone()]);
    // Kaydedilmiş toplantı takvimde 10 dakika uzadı: uç yeni iş sayılmaz.
    let longer = (
        meeting("a", "Trumore haftalık", 0, 70, true),
        "tru".to_string(),
    );
    let p = pieces(&[], &[longer], &classifier, &config, t(-540), t(900));
    assert!(propose(&p, &names, &sheet, &saved).is_empty());
}

#[test]
fn a_separate_short_row_survives_saving_its_neighbour() {
    let (classifier, names, config, sheet) = setup();
    // Toplantıdan önceki 8 dakikalık iş kendi satırıdır: toplantı satırı kaydedilince onun artığı
    // sayılıp kaybolmaz.
    let work = [s("Figma", "Trumore Loyalty — Figma", -8, 0, None)];
    let sync = (
        meeting("a", "Trumore haftalık", 0, 45, false),
        "tru".to_string(),
    );
    let p = pieces(&work, &[sync], &classifier, &config, t(-540), t(900));
    let all = propose(&p, &names, &sheet, &HashMap::new());
    assert_eq!(all.len(), 2);
    let meeting_row = all.iter().find(|e| e.kind == EntryKind::F2F).unwrap();
    let saved = HashMap::from([("tru".to_string(), vec![meeting_row.spans()])]);
    let left = propose(&p, &names, &sheet, &saved);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].spans(), vec![(t(-8), t(0))]);

    // Elle bölünmüş bloğun kısa ikinci yarısı da ilk yarısı kaydedilince kalır.
    let mut split = vec![
        s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
        s("Figma", "Trumore Loyalty — Figma", 60, 70, None),
    ];
    split[1].block_from = Some(t(60));
    let p = pieces(&split, &[], &classifier, &config, t(-540), t(900));
    let all = propose(&p, &names, &sheet, &HashMap::new());
    assert_eq!(all.len(), 2);
    let saved = HashMap::from([("tru".to_string(), vec![all[0].spans()])]);
    assert_eq!(propose(&p, &names, &sheet, &saved), vec![all[1].clone()]);
}

#[test]
fn stale_rows_are_found_and_refreshed() {
    let (classifier, names, config, sheet) = setup();
    let sessions = vec![
        s("Mail", "Rapor", 0, 30, Some("tru")),
        s("Mail", "Rapor", 30, 60, Some("tru")),
    ];
    let p = pieces(&sessions, &[], &classifier, &config, t(-540), t(900));
    let mut row = propose(&p, &names, &sheet, &HashMap::new()).remove(0);
    row.details = "Elle yazıldı".into();
    assert_eq!(stale_hours(&p, &row), None);
    // İlk yarım saat raporda başka projeye alındı.
    let mut moved = sessions.clone();
    moved[0].project_id = Some("kum".into());
    let p = pieces(&moved, &[], &classifier, &config, t(-540), t(900));
    assert_eq!(stale_hours(&p, &row), Some(0.5));
    let fresh = refreshed(&p, &row).unwrap();
    assert_eq!(fresh.start.format("%H:%M").to_string(), "09:30");
    assert_eq!((fresh.hours, fresh.actual_hours), (0.5, Some(0.5)));
    assert_eq!(fresh.details, "Elle yazıldı");
    assert_eq!(fresh.spans(), vec![(t(30), t(60))]);
    // Elle değiştirilen başlangıç korunur.
    let typed = TimesheetEntry {
        start: NaiveTime::from_hms_opt(8, 45, 0).unwrap(),
        ..row.clone()
    };
    assert_eq!(refreshed(&p, &typed).unwrap().start, typed.start);
    // Hepsi gitti: satır kalmaz.
    moved[1].project_id = Some("kum".into());
    let p = pieces(&moved, &[], &classifier, &config, t(-540), t(900));
    assert_eq!(stale_hours(&p, &row), Some(0.0));
    assert_eq!(refreshed(&p, &row), None);
    // Birkaç dakika kaldıysa da satır kalmaz (çeyrek saate yuvarlanmasın).
    let mut few = moved.clone();
    few[1] = s("Mail", "Rapor", 30, 33, Some("tru"));
    let p = pieces(&few, &[], &classifier, &config, t(-540), t(900));
    assert_eq!(refreshed(&p, &row), None);
    // Elle eklenen satır takipten bağımsızdır.
    let manual = TimesheetEntry {
        coverage: Some(Vec::new()),
        ..row
    };
    assert_eq!(stale_hours(&p, &manual), None);
}

#[test]
fn rows_merge_into_one() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore Loyalty — Figma", 0, 10, None),
        s("Figma", "Trumore Rapor — Figma", 60, 70, None),
        s("Figma", "Trumore Loyalty — Figma", 120, 160, None),
        s("us.zoom.xos", "Zoom", 200, 205, Some("tru")),
    ];
    let rows = day(&sessions, &[], &classifier, &names, &config, &sheet);
    assert_eq!(rows.len(), 4);
    // 10 + 10 + 40 + 5 dk: ayrı ayrı yuvarlanınca 0,25 × 3 + 0,75 = 1,50; birleşince 1,00.
    let merged = merge(&rows).unwrap();
    assert_eq!(merged.start.format("%H:%M").to_string(), "09:00");
    assert_eq!(merged.hours, 1.0);
    assert!((merged.worked() - 65.0 / 60.0).abs() < 1e-9);
    assert_eq!(merged.kind, EntryKind::Working);
    assert_eq!(
        merged.details, "Trumore Loyalty; Trumore Rapor",
        "tekrar eden açıklama bir kez"
    );
    assert_eq!(
        merged.spans(),
        vec![
            (t(0), t(10)),
            (t(60), t(70)),
            (t(120), t(160)),
            (t(200), t(205))
        ]
    );
    // Elle değiştirilen saat korunur: toplam saat.
    let mut edited = rows[..2].to_vec();
    edited[1].hours = 1.0;
    assert_eq!(merge(&edited).unwrap().hours, 1.25);
    // Tür, en çok saati olan.
    let mut kinds = rows[..2].to_vec();
    kinds[1].kind = EntryKind::F2F;
    kinds[1].hours = 2.0;
    assert_eq!(merge(&kinds).unwrap().kind, EntryKind::F2F);
    // Eski (aralığı bilinmeyen) satır varsa sonuç da öyle.
    let mut legacy = rows[..2].to_vec();
    legacy[0].coverage = None;
    assert_eq!(merge(&legacy).unwrap().coverage, None);

    assert_eq!(merge(&rows[..1]), Err(MergeError::TooFew));
    let mut other = rows[..2].to_vec();
    other[1].project_id = "sync".into();
    assert_eq!(merge(&other), Err(MergeError::Projects));
    other[1].date = other[1].date.succ_opt().unwrap();
    assert_eq!(merge(&other), Err(MergeError::Days));
}

#[test]
fn file_rows_link_to_exported_rows() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore Loyalty — Figma", 0, 60, None),
        s("Figma", "Trumore Rapor — Figma", 120, 180, None),
        s("Figma", "Trumore Sunum — Figma", 240, 270, None),
    ];
    let rows = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let entries: Vec<&TimesheetEntry> = rows.iter().collect();
    let written: Vec<FileRow> = rows
        .iter()
        .enumerate()
        .map(|(i, e)| FileRow::of(e, i as u32 + 2))
        .collect();
    // Elle girilmiş satır (Kum'da yok) ve dosyada değiştirilmiş açıklama.
    let mut file = written.clone();
    file[1].details = "Aylık rapor (düzeltildi)".into();
    file[1].hours = Some(1.5);
    let manual = FileRow {
        row: 9,
        date: rows[0].date,
        start: NaiveTime::from_hms_opt(16, 0, 0),
        hours: Some(1.0),
        kind: "F2F".into(),
        details: "Atölye".into(),
        ..Default::default()
    };
    file.push(manual.clone());
    // Sunumun başlangıcı dosyada değişti: eşlenmez.
    file[2].start = NaiveTime::from_hms_opt(8, 0, 0);
    file[2].kind = "Online".into();
    file[2].details = "başka".into();
    assert_eq!(
        link_file_rows(&file, &entries),
        [Some(0), Some(1), None, None]
    );
    assert!(written[0].matches(&rows[0]) && !file[1].matches(&rows[1]));
    // Değiştirilen satır Kum'a dosyadaki haliyle geçer; aralıkları korunur.
    let synced = file[1].apply(&rows[1]).unwrap();
    assert_eq!(
        (synced.details.as_str(), synced.hours),
        ("Aylık rapor (düzeltildi)", 1.5)
    );
    assert_eq!(synced.coverage, rows[1].coverage);
    assert!(file[1].matches(&synced));
    // Türü tanınmayan ya da saati boş satır Kum'a geçmez.
    assert!(
        FileRow {
            hours: None,
            ..file[1].clone()
        }
        .apply(&rows[1])
        .is_none()
    );
    // Aynı satır iki kez eşlenmez.
    let twice = vec![written[0].clone(), written[0].clone()];
    assert_eq!(link_file_rows(&twice, &entries), [Some(0), None]);
    // Aynı saatte elle girilmiş başka birimin satırı (türü aynı olsa da) Kum'un satırı değil.
    let other = FileRow {
        division: "Başka birim".into(),
        details: "Başka iş".into(),
        ..written[0].clone()
    };
    assert_eq!(link_file_rows(&[other], &entries), [None]);
    // Birimi dosyada değiştirilmiş (açıklaması aynı) satır hâlâ Kum'un satırıdır: iki kez
    // sayılmaz.
    let moved = FileRow {
        division: "Başka birim".into(),
        ..written[0].clone()
    };
    assert_eq!(link_file_rows(&[moved], &entries), [Some(0)]);
}

#[test]
fn details_summarize_issue_keys_and_main_titles() {
    let m = |n: i64| Duration::minutes(n);
    let one = HashMap::from([("Trumore Loyalty UI/UX".to_string(), m(40))]);
    assert_eq!(describe(one), "Trumore Loyalty UI/UX");
    let many = HashMap::from([
        ("PROJ-12 Ödeme ekranı hatası - Jira".to_string(), m(30)),
        ("[PROJ-15] Sepet tutarı".to_string(), m(20)),
        ("Yeni Sekme".to_string(), m(15)),
        ("main.rs — kum".to_string(), m(25)),
        ("Haberler".to_string(), m(2)),
    ]);
    assert_eq!(
        describe(many),
        "PROJ-12, PROJ-15: Ödeme ekranı hatası - Jira; main.rs — kum; Sepet tutarı"
    );
    assert_eq!(
        describe(HashMap::from([("ABC-1".to_string(), m(5))])),
        "ABC-1"
    );
    assert_eq!(
        describe(HashMap::from([("Yeni Sekme".to_string(), m(5))])),
        ""
    );
    let long = HashMap::from([("x".repeat(400), m(5))]);
    assert_eq!(describe(long).chars().count(), MAX_DETAILS);
    // "COVID-19" gibi sözcükler de anahtara benzer; kabul edilebilir, ama küçük harfli değil.
    assert!(issue_keys("covid-19 ve utf-8").is_empty());
}

#[test]
fn manual_entries_are_face_to_face_and_mapping_applies() {
    let (classifier, names, config, mut sheet) = setup();
    sheet.projects[1].party = Some("Togg".into());
    let mut meeting = s("kum.manual/Workshop", "Workshop", 0, 60, Some("sync"));
    meeting.app_name = "Workshop".into();
    let got = day(&[meeting], &[], &classifier, &names, &config, &sheet);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, EntryKind::F2F);
    assert_eq!(got[0].details, "Workshop");
    assert_eq!(
        (got[0].division.as_str(), got[0].party.as_str()),
        ("Int.Work.Sync.", "Togg")
    );
    assert!((got[0].hours - 1.0).abs() < 1e-9);
    assert_eq!(got[0].actual_hours, Some(1.0));
}

#[test]
fn default_details_fill_only_empty_descriptions() {
    let (classifier, names, config, mut sheet) = setup();
    sheet.projects[0].default_details = Some("  Trumore danışmanlık ".into());
    let sessions = [
        // Başlıktan açıklama çıkar: hazır metin kullanılmaz.
        s("Figma", "Trumore Pitchdeck — Figma", 0, 30, None),
        // Hazır metni olmayan projenin boş açıklaması boş kalır.
        s("us.zoom.xos", "Zoom Meeting", 60, 90, Some("sync")),
        // Toplantı uygulaması: açıklama boş kalır, hazır metin girer.
        s("us.zoom.xos", "Zoom Meeting", 120, 150, Some("tru")),
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let details: Vec<(&str, &str)> = got
        .iter()
        .map(|e| (e.project_id.as_str(), e.details.as_str()))
        .collect();
    assert_eq!(
        details,
        [
            ("tru", "Trumore Pitchdeck"),
            ("sync", ""),
            ("tru", "Trumore danışmanlık")
        ]
    );
}

#[test]
fn mapping_without_default_details_still_parses() {
    let m: ProjectMapping =
        serde_json::from_str(r#"{"projectId":"a","division":"A","party":null}"#).unwrap();
    assert_eq!(m.default_details, None);
    // Aralıkları olmayan (önceki sürümün) kayıt da okunur.
    let e: TimesheetEntry = serde_json::from_str(
        r#"{"date":"2026-10-01","start":"09:00:00","hours":1,"kind":"Working",
                "details":"","party":"","projectId":"a","division":"A"}"#,
    )
    .unwrap();
    assert_eq!(e.coverage, None);
}

#[test]
fn config_keeps_each_project_in_one_sheet() {
    let mut config = TimesheetConfig {
        timesheets: vec![
            Timesheet {
                id: "a".into(),
                projects: vec![mapping("p1", ""), mapping("p2", "")],
                divisions: vec![" Trumore ".into(), "trumore".into(), "".into()],
                ..Default::default()
            },
            Timesheet {
                id: "a".into(),
                projects: vec![mapping("p2", ""), mapping("p3", "")],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    config.normalize();
    let [a, b] = &config.timesheets[..] else {
        panic!("iki çizelge");
    };
    assert_ne!(a.id, b.id);
    assert_eq!(a.divisions, ["Trumore"]);
    assert!(a.includes("p2") && !b.includes("p2") && b.includes("p3"));
    assert_eq!(
        config.timesheet_of("p3").map(|t| t.id.as_str()),
        Some(b.id.as_str())
    );
    assert!(config.timesheet_of("p4").is_none());
}

#[test]
fn assigned_idle_time_is_face_to_face() {
    let (c, names, config, sheet) = setup();
    let mut away = Session::idle(t(0), t(60));
    away.project_id = Some("tru".into());
    let out = day(&[away], &[], &c, &names, &config, &sheet);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, EntryKind::F2F);
    assert_eq!(out[0].hours, 1.0);
}

#[test]
fn browser_meetings_are_online() {
    let (classifier, names, config, sheet) = setup();
    let got = day(
        &[s("com.google.Chrome", "Meet - Trumore weekly", 0, 30, None)],
        &[],
        &classifier,
        &names,
        &config,
        &sheet,
    );
    assert_eq!(got[0].kind, EntryKind::Online);
}

fn meeting(uid: &str, subject: &str, from: i64, to: i64, online: bool) -> Meeting {
    Meeting {
        uid: uid.into(),
        start: t(from),
        end: t(to),
        subject: subject.into(),
        location: String::new(),
        online,
        ..Meeting::default()
    }
}

#[test]
fn calendar_meetings_become_entries_and_replace_tracked_time() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        // 09:00–10:30 Figma; 09:30–10:00 arası toplantıdaydı (ekran paylaşımı).
        s("Figma", "Trumore Loyalty UI/UX — Figma", 0, 90, None),
    ];
    let meetings = [
        (
            meeting("w", "Trumore haftalık", 30, 60, true),
            "tru".to_string(),
        ),
        // Yüz yüze; ilk 15 dakikası öncekiyle çakışıyor (iki kez sayılmaz).
        (
            meeting("f", "Sync atölye", 45, 105, false),
            "sync".to_string(),
        ),
    ];
    let got = day(&sessions, &meetings, &classifier, &names, &config, &sheet);
    let rows: Vec<_> = got
        .iter()
        .map(|e| {
            (
                e.start.format("%H:%M").to_string(),
                (e.worked() * 60.0).round() as i64,
                e.kind,
                e.details.as_str(),
                e.division.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            // Figma 90 dk − toplantılar (09:30–10:30 kapalı) = 30 dk.
            (
                "09:00".to_string(),
                30,
                EntryKind::Working,
                "Trumore Loyalty UI/UX",
                "Trumore"
            ),
            (
                "09:30".to_string(),
                30,
                EntryKind::Online,
                "Trumore haftalık",
                "Trumore"
            ),
            (
                "10:00".to_string(),
                45,
                EntryKind::F2F,
                "Sync atölye",
                "Int.Work.Sync."
            ),
        ]
    );
    // Toplantı başka projeye atanınca kaydedilmiş satırı takipte değişmiş olur.
    let p = pieces(&sessions, &meetings, &classifier, &config, t(-540), t(900));
    let saved_meeting = &got[1];
    assert_eq!(stale_hours(&p, saved_meeting), None);
    let moved = [
        (meetings[0].0.clone(), "kum".to_string()),
        meetings[1].clone(),
    ];
    let p = pieces(&sessions, &moved, &classifier, &config, t(-540), t(900));
    assert_eq!(stale_hours(&p, saved_meeting), Some(0.0));
}

#[test]
fn meeting_project_prefers_assignment_then_rules() {
    let (classifier, _, _, _) = setup();
    let m = meeting("seri", "Trumore weekly", 0, 30, true);
    let mut assigned = HashMap::new();
    assert_eq!(
        meeting_project(&m, &classifier, &assigned),
        MeetingProject::Project("tru".into())
    );
    let other = meeting("x", "1:1", 0, 30, true);
    assert_eq!(
        meeting_project(&other, &classifier, &assigned),
        MeetingProject::Unassigned
    );
    assigned.insert("seri".to_string(), Some("sync".to_string()));
    assigned.insert("x".to_string(), None);
    assert_eq!(
        meeting_project(&m, &classifier, &assigned),
        MeetingProject::Project("sync".into())
    );
    assert_eq!(
        meeting_project(&other, &classifier, &assigned),
        MeetingProject::Ignored
    );
}

fn entry(project: &str, kind: EntryKind, hh: u32, mm: u32, hours: f64) -> TimesheetEntry {
    TimesheetEntry {
        date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        start: NaiveTime::from_hms_opt(hh, mm, 0).unwrap(),
        hours,
        actual_hours: None,
        kind,
        details: String::new(),
        party: String::new(),
        project_id: project.into(),
        division: project.into(),
        coverage: None,
    }
}

#[test]
fn hours_are_rounded_to_quarters_and_actual_is_kept() {
    let (classifier, names, config, sheet) = setup();
    let sessions = [
        s("Figma", "Trumore — Figma", 0, 67, None), // 1 sa 7 dk → 1,00
        s("Figma", "Trumore — Figma", 120, 128, None), // 8 dk → 0,25 (en az)
        s("Figma", "Trumore — Figma", 200, 253, None), // 53 dk → 1,00 (0,88 → 1)
    ];
    let got = day(&sessions, &[], &classifier, &names, &config, &sheet);
    let hours: Vec<(f64, i64)> = got
        .iter()
        .map(|e| (e.hours, (e.worked() * 60.0).round() as i64))
        .collect();
    assert_eq!(hours, [(1.0, 67), (0.25, 8), (1.0, 53)]);
    assert_eq!(round_quarter(0.37), 0.25);
    assert_eq!(round_quarter(0.38), 0.5);
    assert_eq!(round_quarter(2.6), 2.5);
}

#[test]
fn legacy_rows_are_not_proposed_again() {
    use EntryKind::{Online, Working};
    let short = |v: Vec<TimesheetEntry>| -> Vec<(String, String, f64)> {
        v.into_iter()
            .map(|e| {
                let worked = (e.worked() * 100.0).round() / 100.0;
                (e.project_id, e.start.format("%H:%M").to_string(), worked)
            })
            .collect()
    };
    let proposed = [
        entry("a", Working, 9, 10, 0.83),
        entry("a", Working, 13, 0, 2.0),
        entry("a", Online, 11, 0, 0.5),
        entry("b", Working, 10, 0, 1.0),
    ];
    // Hiç eski satır yoksa öneriler aynen.
    assert_eq!(without_legacy(&proposed, &[]).len(), 4);
    // Aktarırken 09:10 kaydının saati 09:00'a çekilmiş ve 0,75'e yuvarlanmış (kalan 0,08
    // yuvarlama artığı); 13:00 kaydı aktarıldığında 1 saatti, sonra 2 saate uzadı: yalnızca
    // uzayan saat önerilir. Toplantı ve b projesi aktarılmadı: aynen kalır.
    let exported = [
        entry("a", Working, 9, 0, 0.75),
        entry("a", Working, 13, 0, 1.0),
    ];
    assert_eq!(
        short(without_legacy(&proposed, &exported)),
        [
            ("b", "10:00", 1.0),
            ("a", "11:00", 0.5),
            ("a", "14:00", 1.0)
        ]
        .map(|(p, t, h)| (p.to_string(), t.to_string(), h))
    );
    // Türü elle değiştirilmiş toplantı satırı (Online → F2F) yine kendi toplantısını kapsar.
    let exported = [entry("a", EntryKind::F2F, 11, 0, 0.5)];
    assert!(
        without_legacy(&proposed, &exported)
            .iter()
            .all(|e| e.kind != Online)
    );
    // Kalan 15 dakikadan kısaysa (yuvarlama artığı) önerilmez.
    let exported = [entry("a", Working, 9, 0, 2.75)];
    assert!(
        without_legacy(&proposed, &exported)
            .iter()
            .all(|e| e.project_id != "a" || e.kind != Working)
    );
    // Tamamı aktarılmış gün: yeni bir şey yok.
    assert!(without_legacy(&proposed, &proposed).is_empty());

    // Kısmen düşülen önerinin aralıkları da düşülen süre kadar kısalır.
    let mut long = entry("a", Working, 9, 0, 2.0);
    long.actual_hours = Some(2.0);
    long.coverage = Some(to_coverage(&[(t(0), t(60)), (t(90), t(150))]));
    let rest = without_legacy(&[long], &[entry("a", Working, 9, 0, 1.25)]);
    assert_eq!(rest[0].spans(), vec![(t(105), t(150))]);
    assert_eq!(rest[0].start.format("%H:%M").to_string(), "10:15");
}

#[test]
fn spans_coalesce() {
    assert_eq!(
        coalesce(vec![
            (t(30), t(40)),
            (t(0), t(10)),
            (t(10), t(20)),
            (t(35), t(50)),
            (t(60), t(60)),
        ]),
        vec![(t(0), t(20)), (t(30), t(50))]
    );
    let spans = vec![(t(0), t(20)), (t(30), t(50))];
    assert_eq!(from_coverage(&to_coverage(&spans)), spans);
    assert_eq!(total(&spans), Duration::minutes(40));
}
