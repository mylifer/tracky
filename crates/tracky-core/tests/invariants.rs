//! Takvim, rapor ve zaman çizelgesi arasında her zaman doğru olması gerekenler.
//!
//! Rastgele (tohumdan tekrarlanabilir) günler üretilir: kısa pencere geçişleri, molalar, iki
//! bilgisayarın çakışan kayıtları, elle atamalar, projeye atanmış boşta süre, elle bölünmüş
//! bloklar, gece yarısını aşan oturumlar ve takvim toplantıları. Her gün üç halde denetlenir:
//! taze; satırların bir kısmı kaydedilmiş, düzenlenmiş, gizlenmiş ve gönderilmiş; ardından işin
//! bir kısmı raporda başka projeye ya da projesize alınmış.
//!
//! Varsayılan 80 gün denetlenir; daha genişi için `KUM_SEEDS=3000`. Bozulan tek bir gün
//! `SEED=588 cargo test -p tracky-core --test invariants one_seed -- --ignored` ile yinelenir.
//!
//! Aynı denetim gerçek veride de çalışır (veritabanı önce geçici bir kopyaya alınır):
//!
//! ```sh
//! KUM_DB=~/Library/Application\ Support/com.kum.app/kum.db \
//! KUM_ICS=~/Library/Application\ Support/com.kum.app/calendar.ics \
//!   cargo test -p tracky-core --test invariants -- --ignored --nocapture
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Days, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};
use tracky_core::calendar::Calendar;
use tracky_core::classify::{NO_PROJECT, Rule, RuleField, Tag, TagKind};
use tracky_core::client_report::ReportSource;
use tracky_core::model::{IDLE_APP_ID, Session};
use tracky_core::store::{DayRow, Store};
use tracky_core::timesheet::{
    EntryKind, Interval, MERGE_GAP, MIN_ENTRY, Meeting, ProjectMapping, STALE_SLACK, Timesheet,
    TimesheetConfig, coalesce, round_quarter, total,
};
use uuid::Uuid;

/// Yerel gün başı (yaz saatine geçilen gün gece yarısı yoksa ilk geçerli saat).
fn midnight(d: NaiveDate) -> DateTime<Utc> {
    (0..4)
        .find_map(|h| {
            Local
                .from_local_datetime(&d.and_hms_opt(h, 0, 0)?)
                .earliest()
        })
        .expect("günün başı")
        .with_timezone(&Utc)
}

fn hm(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M:%S").to_string()
}

/// İki aralık kümesinin farkı.
fn minus(a: &[Interval], b: &[Interval]) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(x, y) in a {
        let mut parts = vec![(x, y)];
        for &(c, d) in b {
            parts = parts
                .into_iter()
                .flat_map(|(p, q)| {
                    if d <= p || c >= q {
                        vec![(p, q)]
                    } else {
                        [(p, c.max(p)), (d.min(q), q)]
                            .into_iter()
                            .filter(|(s, e)| e > s)
                            .collect()
                    }
                })
                .collect();
        }
        out.extend(parts);
    }
    coalesce(out)
}

fn intersect(a: &[Interval], b: &[Interval]) -> Vec<Interval> {
    let mut out = Vec::new();
    for &(x, y) in a {
        for &(c, d) in b {
            let (s, e) = (x.max(c), y.min(d));
            if e > s {
                out.push((s, e));
            }
        }
    }
    coalesce(out)
}

/// İki aralık kümesi (en çok `slack` farkla) aynı mı.
fn same_spans(a: &[Interval], b: &[Interval], slack: Duration) -> bool {
    total(&minus(a, b)) <= slack && total(&minus(b, a)) <= slack
}

/// Bir günün bütün değişmezleri; bozulanlar metin olarak döner. `fresh`: günde kaydedilmiş satır
/// ve elle bölünmüş blok yok (her iş kümesi ya tam bir satırdır ya da satır olamayacak kadar
/// kısadır). `overlap`: iki satırın çakışabileceği en uzun süre (kaydedildikten sonra işi
/// `STALE_SLACK`'ten az değişen satır "takipte değişti" sayılmaz).
fn check_day(
    store: &Store,
    date: NaiveDate,
    meetings: &[Meeting],
    fresh: bool,
    overlap: Duration,
) -> Vec<String> {
    let mut bad = Vec::new();
    let (from, to) = (midnight(date), midnight(date + Days::new(1)));

    // --- Rapor: toplamlar ve bloklar -------------------------------------------------------
    let r = store.report(from, to, &[from], true).unwrap();
    let total_s = r.total_seconds;
    let sums = [
        (
            "kategori",
            r.categories.iter().map(|b| b.seconds).sum::<i64>(),
            r.categories.len(),
        ),
        (
            "proje",
            r.projects.iter().map(|b| b.seconds).sum(),
            r.projects.len(),
        ),
        (
            "uygulama",
            r.apps.iter().map(|b| b.seconds).sum(),
            r.apps.len(),
        ),
    ];
    for (what, sum, n) in sums {
        // Her kova milisaniyeden saniyeye aşağı yuvarlanır: toplam en çok kova sayısı kadar az.
        if sum > total_s || total_s - sum > n as i64 {
            bad.push(format!(
                "rapor: {what} toplamı {sum} sn, gün toplamı {total_s} sn"
            ));
        }
    }
    if r.days.len() != 1 || (r.days[0].seconds - total_s).abs() > 1 {
        bad.push(format!(
            "rapor: gün kovası {:?} ≠ toplam {total_s}",
            r.days.first().map(|d| d.seconds)
        ));
    }
    // Aynı pencerenin 5 sn'ye kadar boşlukla ardışık kayıtları ekranda tek parça görünür: pencere
    // toplamı gün toplamını en çok pencere başına 5 sn aşar, hiç eksik kalmaz.
    let windows_ms: i64 = r
        .windows
        .iter()
        .map(|w| (w.end - w.start).num_milliseconds())
        .sum();
    if windows_ms / 1000 < total_s - 1 || windows_ms / 1000 > total_s + 5 * r.windows.len() as i64 {
        bad.push(format!(
            "rapor: pencerelerin toplamı {} sn, gün toplamı {total_s}",
            windows_ms / 1000
        ));
    }
    for w in r.windows.windows(2) {
        if w[1].start < w[0].end {
            bad.push(format!(
                "rapor: pencereler çakışıyor {}–{} / {}",
                hm(w[0].start),
                hm(w[0].end),
                hm(w[1].start)
            ));
        }
    }
    let blocks = &r.work.blocks;
    let ctx = store.timesheet_context().unwrap();
    let classifier = ctx.classifier();
    let work_sessions = store.merged_sessions_between(from, to).unwrap();
    let active: i64 = blocks.iter().map(|b| b.active_seconds).sum();
    if active > total_s || total_s - active > blocks.len() as i64 {
        bad.push(format!(
            "blok: etkin toplam {active} sn ≠ gün toplamı {total_s} sn"
        ));
    }
    for (i, b) in blocks.iter().enumerate() {
        if b.start < from || b.end > to || b.end < b.start {
            bad.push(format!(
                "blok {}–{}: gün dışında ya da ters",
                hm(b.start),
                hm(b.end)
            ));
        }
        if b.active_seconds > (b.end - b.start).num_seconds() + 1 {
            bad.push(format!(
                "blok {}–{}: etkin süre aralıktan uzun",
                hm(b.start),
                hm(b.end)
            ));
        }
        if let Some(next) = blocks.get(i + 1)
            && next.start < b.end
        {
            bad.push(format!(
                "blok {}–{} ile {} çakışıyor",
                hm(b.start),
                hm(b.end),
                hm(next.start)
            ));
        }
        // Bloğun adı (projesi), içindeki işin en az yarısının projesi.
        let mut by_project: HashMap<Option<String>, i64> = HashMap::new();
        for x in &work_sessions {
            let ms = (x.ended_at.min(b.end) - x.started_at.max(b.start)).num_milliseconds();
            if ms > 0 {
                *by_project
                    .entry(classifier.classify(x).project)
                    .or_default() += ms;
            }
        }
        let in_block: i64 = by_project.values().sum();
        match &b.project_id {
            Some(p)
                if by_project.get(&Some(p.clone())).copied().unwrap_or(0) * 2 < in_block - 1000 =>
            {
                bad.push(format!(
                    "blok {}–{}: projesi pencerelerinin yarısından azında",
                    hm(b.start),
                    hm(b.end)
                ));
            }
            None => {
                if let Some((p, ms)) = by_project
                    .iter()
                    .find(|(p, ms)| p.is_some() && **ms * 2 > in_block + 1000)
                {
                    bad.push(format!("blok {}–{}: pencerelerin yarısı {p:?} projesinde ({ms} ms) ama blok projesiz", hm(b.start), hm(b.end)));
                }
            }
            _ => {}
        }
    }

    // --- Zaman çizelgesi parçaları ---------------------------------------------------------
    let pieces = store.timesheet_pieces(&ctx, from, to, meetings).unwrap();
    let sessions = store.merged_sessions_between(from, to).unwrap();
    let work = coalesce(
        sessions
            .iter()
            .map(|s| (s.started_at.max(from), s.ended_at.min(to)))
            .filter(|(a, b)| b > a)
            .collect(),
    );
    let mut sorted: Vec<_> = pieces.iter().collect();
    sorted.sort_by_key(|p| (p.start, p.end));
    for w in sorted.windows(2) {
        if w[1].start < w[0].end {
            bad.push(format!(
                "parça: {}–{} ile {}–{} çakışıyor (aynı saat iki kez)",
                hm(w[0].start),
                hm(w[0].end),
                hm(w[1].start),
                hm(w[1].end)
            ));
        }
    }
    for p in pieces.iter() {
        if p.start < from || p.end > to || p.end <= p.start {
            bad.push(format!(
                "parça {}–{}: gün dışında ya da boş",
                hm(p.start),
                hm(p.end)
            ));
        }
        if p.meeting.is_none() && !minus(&[(p.start, p.end)], &work).is_empty() {
            bad.push(format!(
                "parça {}–{}: takip edilmeyen süre",
                hm(p.start),
                hm(p.end)
            ));
        }
        if p.meeting.is_none() {
            // Parçanın projesi oturumunun projesi ya da (atanmamışsa) içinde geçtiği bloğun projesi.
            let s = sessions
                .iter()
                .find(|s| s.started_at <= p.start && p.end <= s.ended_at);
            let own = s.and_then(|s| ctx.classifier().classify(s).project);
            let block = blocks
                .iter()
                .find(|b| b.start <= p.start && p.end <= b.end)
                .and_then(|b| b.project_id.clone());
            let ok = match (&own, s) {
                (Some(o), _) => *o == p.project,
                (None, Some(s)) => {
                    s.project_id.as_deref() != Some(NO_PROJECT)
                        && block.as_deref() == Some(p.project.as_str())
                }
                (None, None) => false,
            };
            if !ok {
                bad.push(format!(
                    "parça {}–{}: projesi {} oturumun ({own:?}) ya da bloğun ({block:?}) değil",
                    hm(p.start),
                    hm(p.end),
                    p.project
                ));
            }
        }
    }
    let mut piece_spans: HashMap<&str, Vec<Interval>> = HashMap::new();
    for p in pieces.iter() {
        piece_spans
            .entry(p.project.as_str())
            .or_default()
            .push((p.start, p.end));
    }
    let piece_spans: HashMap<&str, Vec<Interval>> = piece_spans
        .into_iter()
        .map(|(k, v)| (k, coalesce(v)))
        .collect();

    // --- Satırlar ---------------------------------------------------------------------------
    let mut rows: Vec<(String, DayRow)> = Vec::new();
    for sheet in &ctx.config.timesheets {
        for row in store
            .timesheet_day(&ctx, sheet, date, &pieces)
            .unwrap()
            .rows
        {
            let e = &row.entry;
            let label = format!(
                "satır [{} {} {} {:.2} sa]",
                if row.id.is_none() {
                    "canlı"
                } else if row.exported {
                    "gönderilmiş"
                } else {
                    "kayıtlı"
                },
                e.start.format("%H:%M:%S"),
                e.project_id,
                e.hours
            );
            if e.date != date {
                bad.push(format!("{label}: başka günün satırı ({})", e.date));
            }
            if !row.exported && !sheet.includes(&e.project_id) {
                bad.push(format!("{label}: projesi bu çizelgeye bağlı değil"));
            }
            if !(e.hours > 0.0 && e.hours.is_finite()) {
                bad.push(format!("{label}: saat geçersiz"));
            }
            let spans = e.spans();
            for w in spans.windows(2) {
                if w[1].0 < w[0].1 {
                    bad.push(format!("{label}: aralıkları sırasız ya da çakışık"));
                }
            }
            if spans.iter().any(|s| s.0 < from || s.1 > to || s.1 <= s.0) {
                bad.push(format!("{label}: aralığı gün dışında ya da boş"));
            }
            if row.id.is_none() {
                let worked = total(&spans).num_milliseconds() as f64 / 3_600_000.0;
                let actual = e.actual_hours.unwrap_or(f64::NAN);
                if (actual - worked).abs() > 1.0 / 3600.0 {
                    bad.push(format!(
                        "{label}: gerçek süre {actual:.4} ≠ aralıkların {worked:.4}"
                    ));
                }
                if (e.hours - round_quarter(actual)).abs() > 1e-9 {
                    bad.push(format!(
                        "{label}: saat yuvarlanmış gerçek süre değil ({})",
                        round_quarter(actual)
                    ));
                }
                if total(&spans) < MIN_ENTRY {
                    bad.push(format!("{label}: MIN_ENTRY'den kısa"));
                }
                match spans.first() {
                    Some(first) => {
                        let start = first
                            .0
                            .with_timezone(&Local)
                            .time()
                            .with_nanosecond(0)
                            .unwrap();
                        if start != e.start {
                            bad.push(format!("{label}: başlangıç ilk aralık değil ({start})"));
                        }
                    }
                    None => bad.push(format!("{label}: takipten gelen satırın aralığı yok")),
                }
                let own = piece_spans
                    .get(e.project_id.as_str())
                    .map_or(&[][..], |v| v.as_slice());
                let outside = minus(&spans, own);
                if !outside.is_empty() {
                    bad.push(format!(
                        "{label}: projesinin olmayan {} sn kapsıyor",
                        total(&outside).num_seconds()
                    ));
                }
            }
            rows.push((label, row));
        }
    }
    // Aynı saat iki satıra yazılmaz. Kaydedildikten sonra işi başka projeye alınan satır
    // "takipte değişti" olarak işaretlenir (güncellenmeden gönderilmez, gönderilmişse dosyadaki
    // satırı da güncellenir); yeni projenin satırı yalnızca onunla çakışabilir.
    for i in 0..rows.len() {
        for j in i + 1..rows.len() {
            if rows[i].1.stale.is_some() || rows[j].1.stale.is_some() {
                continue;
            }
            let both = intersect(
                &coalesce(rows[i].1.entry.spans()),
                &coalesce(rows[j].1.entry.spans()),
            );
            if total(&both) > overlap {
                bad.push(format!(
                    "{} ile {} {} sn çakışıyor (çift yazım)",
                    rows[i].0,
                    rows[j].0,
                    total(&both).num_seconds()
                ));
            }
        }
    }

    // --- Taze gün: her iş kümesi ya bir satır ya da satır olamayacak kadar kısa ----------------
    if fresh {
        for sheet in &ctx.config.timesheets {
            for mapping in &sheet.projects {
                let p = mapping.project_id.as_str();
                // Satırların ölçtüğü gibi: oturum parçaları MERGE_GAP'e kadar boşlukla bir küme,
                // her toplantı ayrı küme.
                let mut clusters: Vec<Vec<Interval>> = Vec::new();
                let mut meeting_clusters: HashMap<usize, Vec<Interval>> = HashMap::new();
                let mut own: Vec<_> = pieces.iter().filter(|x| x.project == p).collect();
                own.sort_by_key(|x| x.start);
                let mut end = None;
                for x in own {
                    if let Some(m) = x.meeting {
                        meeting_clusters
                            .entry(m)
                            .or_default()
                            .push((x.start, x.end));
                        continue;
                    }
                    match (clusters.last_mut(), end) {
                        (Some(c), Some(e)) if x.start - e <= MERGE_GAP => c.push((x.start, x.end)),
                        _ => clusters.push(vec![(x.start, x.end)]),
                    }
                    end = Some(end.map_or(x.end, |e: DateTime<Utc>| e.max(x.end)));
                }
                clusters.extend(meeting_clusters.into_values());
                let covered: Vec<Interval> = coalesce(
                    rows.iter()
                        .filter(|(_, r)| r.entry.project_id == p)
                        .flat_map(|(_, r)| r.entry.spans())
                        .collect(),
                );
                for c in clusters {
                    let c = coalesce(c);
                    let left = minus(&c, &covered);
                    if left.is_empty() {
                        continue;
                    }
                    if !same_spans(&left, &c, Duration::seconds(1)) {
                        bad.push(format!(
                            "taze: {p} kümesi {}–{} satıra yarım girmiş",
                            hm(c[0].0),
                            hm(c[c.len() - 1].1)
                        ));
                    } else if total(&c) >= MIN_ENTRY {
                        bad.push(format!(
                            "taze: {p} kümesi {}–{} ({} sn) satır olmamış (kayıp iş)",
                            hm(c[0].0),
                            hm(c[c.len() - 1].1),
                            total(&c).num_seconds()
                        ));
                    }
                }
            }
        }
    }

    // --- Müşteri raporu (takip edilen süre) proje toplamlarıyla aynı ----------------------------
    let cr = store
        .client_report(
            vec![date],
            &[from, to],
            None,
            Some(ReportSource::Tracked),
            meetings,
        )
        .unwrap();
    for b in &r.projects {
        let Some(id) = &b.id else { continue };
        let got = cr
            .rows
            .iter()
            .find(|x| &x.project_id == id)
            .map_or(0.0, |x| x.hours[0]);
        // Etiketi silinmiş proje müşteri raporunda yoktur.
        if store.tags().unwrap().iter().any(|t| &t.id == id)
            && (got * 3600.0 - b.seconds as f64).abs() > 1.5
        {
            bad.push(format!(
                "müşteri raporu: {id} {got:.4} sa ≠ rapor {} sn",
                b.seconds
            ));
        }
    }
    bad
}

/// Gün gün raporun blokları, iki günlük raporunkiyle aynı (hafta görünümü = gün görünümleri).
fn check_span(store: &Store, first: NaiveDate, days: u64) -> Vec<String> {
    let starts: Vec<_> = (0..=days).map(|i| midnight(first + Days::new(i))).collect();
    let n = days as usize;
    let all = store
        .report(starts[0], starts[n], &starts[..n], true)
        .unwrap();
    let mut each = Vec::new();
    let mut total_s = 0;
    for i in 0..n {
        let r = store
            .report(starts[i], starts[i + 1], &[starts[i]], true)
            .unwrap();
        total_s += r.total_seconds;
        each.extend(r.work.blocks);
    }
    let mut bad = Vec::new();
    if (all.total_seconds - total_s).abs() > n as i64 {
        bad.push(format!(
            "dönem: toplam {} ≠ günlerin toplamı {total_s}",
            all.total_seconds
        ));
    }
    // Bilgisayar dağılımı yalnızca aralıkta birden çok bilgisayar varsa dolar: tek bilgisayarlı
    // günün görünümünde boş, iki bilgisayarlı haftanınkinde dolu olabilir.
    let strip = |bs: &[tracky_core::blocks::WorkBlock]| {
        bs.iter()
            .cloned()
            .map(|mut b| {
                b.devices.clear();
                b
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (strip(&all.work.blocks), strip(&each));
    if a != b {
        let diff = a.iter().zip(&b).find(|(x, y)| x != y);
        bad.push(format!(
            "dönem: bloklar gün görünümünden farklı ({} ↔ {}): {diff:?}",
            a.len(),
            b.len()
        ));
    }
    bad
}

// --- Rastgele günler ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
    fn ms(&mut self, lo: i64, hi: i64) -> Duration {
        Duration::milliseconds(lo + self.below((hi - lo).max(1) as u64) as i64)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

const APPS: &[(&str, &str, Option<&str>)] = &[
    ("com.microsoft.VSCode", "Code", Some("dev")),
    ("com.figma.Desktop", "Figma", Some("dev")),
    ("com.tinyspeck.slackmacgap", "Slack", Some("comm")),
    ("us.zoom.xos", "zoom.us", Some("comm")),
    ("com.apple.TV", "TV", Some("fun")),
    ("org.mozilla.firefox", "Firefox", None),
];
const TITLES: &[&str] = &[
    "Alpha ekranları",
    "Alpha — Figma",
    "Beta raporu PROJ-12",
    "Gamma notları",
    "Delta planı",
    "Zoom Meeting",
    "Gelen kutusu",
    "New Tab",
    "",
    "Haberler",
];

struct Day {
    store: Store,
    date: NaiveDate,
    meetings: Vec<Meeting>,
    /// Elle bölünmüş blok var.
    split: bool,
    projects: Vec<String>,
}

fn random_day(seed: u64) -> Day {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let store = Store::open_in_memory().unwrap();
    let tag = |id: &str, kind, name: &str| Tag {
        id: id.into(),
        kind,
        name: name.into(),
        color: 1,
    };
    for (i, c) in ["dev", "comm", "fun"].iter().enumerate() {
        store
            .upsert_tag(&tag(c, TagKind::Category, c), i as i64)
            .unwrap();
    }
    for (i, (id, word)) in [
        ("alpha", "Alpha"),
        ("beta", "Beta"),
        ("gamma", "Gamma"),
        ("delta", "Delta"),
    ]
    .iter()
    .enumerate()
    {
        store
            .upsert_tag(&tag(id, TagKind::Project, word), 10 + i as i64)
            .unwrap();
        store
            .upsert_rule(&Rule {
                id: format!("r-{id}"),
                tag_id: id.to_string(),
                field: RuleField::Title,
                pattern: word.to_string(),
            })
            .unwrap();
    }
    for (app, _, cat) in APPS {
        if let Some(cat) = cat {
            store.assign_app_category(app, Some(cat)).unwrap();
        }
    }
    let mapping = |p: &str| ProjectMapping {
        project_id: p.into(),
        division: String::new(),
        party: None,
        default_details: None,
    };
    store
        .save_timesheet_config(&TimesheetConfig {
            timesheets: vec![
                Timesheet {
                    id: "togg".into(),
                    default_party: "ADBA".into(),
                    projects: vec![mapping("alpha"), mapping("beta")],
                    ..Default::default()
                },
                Timesheet {
                    id: "other".into(),
                    projects: vec![mapping("delta")],
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    // 2026 Mart'ın rastgele bir günü (geçmiş: süren iş yok).
    let date = NaiveDate::from_ymd_opt(2026, 3, 1 + rng.below(28) as u32).unwrap();
    let (from, to) = (midnight(date), midnight(date + Days::new(1)));
    // Bazen önceki günden taşan oturumla başlar, bazen ertesi güne taşar.
    let mut t = if rng.chance(0.1) {
        from - rng.ms(1_000, 3_600_000)
    } else {
        from + Duration::hours(6) + rng.ms(0, 3 * 3_600_000)
    };
    let end = if rng.chance(0.15) {
        to + Duration::hours(1)
    } else {
        from + Duration::hours(20)
    };
    let mut split = false;
    let mut last_end = t;
    while t < end {
        let len = match rng.below(100) {
            0..50 => rng.ms(200, 5_000),
            50..80 => rng.ms(5_000, 120_000),
            80..95 => rng.ms(120_000, 1_200_000),
            _ => rng.ms(1_200_000, 3_600_000),
        };
        let (app_id, app_name, _) = *rng.pick(APPS);
        let mut s = Session {
            id: Uuid::new_v4(),
            app_id: app_id.into(),
            app_name: app_name.into(),
            title: rng.pick(TITLES).to_string(),
            url: None,
            domain: None,
            started_at: t,
            ended_at: t + len,
            category_id: None,
            project_id: None,
            block_from: None,
        };
        // İkinci bilgisayar: öncekiyle çakışan kayıt.
        if rng.chance(0.03) {
            s.started_at = (last_end - rng.ms(1_000, 600_000)).max(from - Duration::hours(1));
        }
        if rng.chance(0.05) {
            s.project_id = Some(
                rng.pick(&["alpha", "beta", "gamma", NO_PROJECT])
                    .to_string(),
            );
        }
        if rng.chance(0.03) {
            s.app_id = IDLE_APP_ID.into();
            s.app_name = "Boşta".into();
            s.title = String::new();
            if rng.chance(0.5) {
                s.project_id = Some("alpha".into());
            }
        }
        if rng.chance(0.02) {
            s.block_from = Some(s.started_at);
            split = true;
        }
        last_end = last_end.max(s.ended_at);
        store.upsert_session(&s).unwrap();
        let gap = match rng.below(100) {
            0..70 => rng.ms(0, 3_000),
            70..85 => rng.ms(3_000, 60_000),
            85..95 => rng.ms(60_000, 600_000),
            _ => rng.ms(600_000, 2_400_000),
        };
        t = s.ended_at + gap;
    }

    let mut meetings = Vec::new();
    for i in 0..rng.below(4) {
        let start = from + Duration::hours(8) + rng.ms(0, 9 * 3_600_000);
        meetings.push(Meeting {
            uid: format!("m{i}"),
            start,
            end: start + rng.ms(10 * 60_000, 90 * 60_000),
            subject: rng
                .pick(&[
                    "Alpha sync",
                    "Beta review",
                    "Delta sync",
                    "Kahve",
                    "Gamma demo",
                ])
                .to_string(),
            online: rng.chance(0.6),
            ..Default::default()
        });
    }
    Day {
        store,
        date,
        meetings,
        split,
        projects: vec!["alpha".into(), "beta".into(), "delta".into()],
    }
}

/// Günün bütün çizelgelerindeki satırları.
fn rows_of(store: &Store, date: NaiveDate, meetings: &[Meeting]) -> Vec<(Timesheet, DayRow)> {
    let ctx = store.timesheet_context().unwrap();
    let (from, to) = (midnight(date), midnight(date + Days::new(1)));
    let pieces = store.timesheet_pieces(&ctx, from, to, meetings).unwrap();
    ctx.config
        .timesheets
        .iter()
        .flat_map(|s| {
            store
                .timesheet_day(&ctx, s, date, &pieces)
                .unwrap()
                .rows
                .into_iter()
                .map(move |r| (s.clone(), r))
        })
        .collect()
}

fn covered_by_project(rows: &[(Timesheet, DayRow)]) -> HashMap<String, Vec<Interval>> {
    let mut out: HashMap<String, Vec<Interval>> = HashMap::new();
    for (_, r) in rows {
        out.entry(r.entry.project_id.clone())
            .or_default()
            .extend(r.entry.spans());
    }
    out.into_iter().map(|(k, v)| (k, coalesce(v))).collect()
}

fn run_seed(seed: u64) -> Vec<String> {
    let day = random_day(seed);
    let (store, date, meetings) = (&day.store, day.date, &day.meetings[..]);
    let mut bad: Vec<String> = Vec::new();
    let mut fail =
        |phase: &str, v: Vec<String>| bad.extend(v.into_iter().map(|m| format!("[{phase}] {m}")));

    // 1. Taze gün.
    let exact = Duration::seconds(1);
    fail("taze", check_day(store, date, meetings, !day.split, exact));
    fail(
        "taze",
        check_day(store, date + Days::new(1), &[], !day.split, exact),
    );
    fail("taze", check_span(store, date - Days::new(1), 3));

    // 2. Satırların bir kısmı kaydedilir (bazısı düzenlenerek), bazısı gizlenir, kaydedilenlerin
    //    bir kısmı gönderilir. Takip değişmediği için kapsanan süre yalnızca gizlenenler kadar
    //    azalır; hiçbir satır "takipte değişti" olmaz.
    let mut rng = Rng(seed ^ 0xDEAD_BEEF);
    let before = rows_of(store, date, meetings);
    let mut dismissed_by: HashMap<String, Vec<Interval>> = HashMap::new();
    // Gizlenen toplantı satırı: toplantı yapılmamış sayılır, o saatteki iş yeniden önerilebilir.
    let mut dismissed_meetings: Vec<Interval> = Vec::new();
    let mut saved: Vec<(String, Timesheet)> = Vec::new();
    for (sheet, row) in &before {
        if row.id.is_some() {
            continue;
        }
        match rng.below(10) {
            0..5 => {
                let mut e = row.entry.clone();
                if rng.chance(0.5) {
                    e.details = format!("{} (düzenlendi)", e.details);
                }
                if rng.chance(0.3) {
                    e.hours += 0.25;
                }
                saved.push((store.save_timesheet_entry(None, &e).unwrap(), sheet.clone()));
            }
            5..7 => {
                store.dismiss_timesheet_entry(None, &row.entry).unwrap();
                let spans = row.entry.spans();
                let of_meeting = meetings.iter().any(|m| {
                    let kind = if m.online {
                        EntryKind::Online
                    } else {
                        EntryKind::F2F
                    };
                    row.entry.kind == kind && spans.iter().all(|&(a, b)| m.start <= a && b <= m.end)
                });
                if of_meeting {
                    dismissed_meetings.extend(spans);
                }
                dismissed_by
                    .entry(row.entry.project_id.clone())
                    .or_default()
                    .extend(row.entry.spans());
            }
            _ => {}
        }
    }
    let mut exported = Vec::new();
    for (id, sheet) in &saved {
        if rng.chance(0.5) {
            store
                .mark_timesheet_exported(std::slice::from_ref(id), Utc::now(), &sheet.id, "")
                .unwrap();
            exported.push(id.clone());
        }
    }
    fail("kayıt", check_day(store, date, meetings, false, exact));
    let after = rows_of(store, date, meetings);
    let (was, now) = (covered_by_project(&before), covered_by_project(&after));
    for p in &day.projects {
        let expect = minus(
            was.get(p).map_or(&[][..], |v| v),
            dismissed_by.get(p).map_or(&[][..], |v| v),
        );
        let got = now.get(p).cloned().unwrap_or_default();
        // Kaydedilen ya da gizlenmeyen iş satırlarda kalır (kayıp iş yok). Fazlası yalnızca gizlenen
        // toplantının saatinde ya da onunla aynı satıra giren (eskiden satır olamayacak kadar kısa)
        // işte.
        let lost = minus(&expect, &got);
        let reopened = coalesce(dismissed_meetings.clone());
        let allowed: Vec<Interval> = coalesce(
            after
                .iter()
                .map(|(_, r)| coalesce(r.entry.spans()))
                .filter(|spans| !intersect(spans, &reopened).is_empty())
                .flatten()
                .chain(reopened.iter().copied())
                .collect(),
        );
        let extra = minus(&minus(&got, &expect), &allowed);
        if total(&lost) > Duration::seconds(1) || total(&extra) > Duration::seconds(1) {
            fail(
                "kayıt",
                vec![format!(
                    "{p}: kaydet/gizle sonrası kapsanan süre değişti: kaybolan {} sn ({}), yeni {} sn",
                    total(&lost).num_seconds(),
                    lost.iter()
                        .map(|(a, b)| format!("{}–{}", hm(*a), hm(*b)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    total(&extra).num_seconds()
                )],
            );
        }
    }
    for (_, r) in &after {
        if r.stale.is_some() {
            fail(
                "kayıt",
                vec![format!(
                    "satır {} takip değişmeden 'takipte değişti'",
                    r.entry.start
                )],
            );
        }
    }
    let exported_before: HashMap<String, _> = after
        .iter()
        .filter(|(_, r)| r.exported)
        .map(|(_, r)| (r.id.clone().unwrap(), r.entry.clone()))
        .collect();

    // 3. İşin bir kısmı raporda başka projeye ya da projesize alınır.
    let (from, to) = (midnight(date), midnight(date + Days::new(1)));
    let sessions = store.sessions_between(from, to).unwrap();
    let moved: Vec<Uuid> = sessions
        .iter()
        .filter(|_| rng.chance(0.15))
        .map(|s| s.id)
        .collect();
    let target = *rng.pick(&[NO_PROJECT, "beta", "gamma"]);
    store.set_project_for(&moved, Some(target)).unwrap();
    fail(
        "değişiklik",
        check_day(store, date, meetings, false, STALE_SLACK),
    );
    let ctx = store.timesheet_context().unwrap();
    let pieces = store.timesheet_pieces(&ctx, from, to, meetings).unwrap();
    for (_, r) in rows_of(store, date, meetings) {
        let Some(id) = &r.id else { continue };
        if let Some(e) = exported_before.get(id) {
            if *e != r.entry {
                fail(
                    "değişiklik",
                    vec![format!("gönderilmiş satır {} değişti", e.start)],
                );
            }
            continue;
        }
        if exported_before.contains_key(id) || r.exported {
            continue;
        }
        let spans = coalesce(r.entry.spans());
        let own: Vec<Interval> = coalesce(
            pieces
                .iter()
                .filter(|p| p.project == r.entry.project_id)
                .map(|p| (p.start, p.end))
                .collect(),
        );
        let lost = total(&spans) - total(&intersect(&spans, &own));
        if lost > STALE_SLACK + Duration::seconds(1) && r.stale.is_none() {
            fail(
                "değişiklik",
                vec![format!(
                    "satır {}: {} sn işi gitti ama 'takipte değişti' değil",
                    r.entry.start,
                    lost.num_seconds()
                )],
            );
        }
        if lost.is_zero() && r.stale.is_some() {
            fail(
                "değişiklik",
                vec![format!(
                    "satır {}: işi değişmedi ama 'takipte değişti'",
                    r.entry.start
                )],
            );
        }
    }
    bad
}

#[test]
fn random_days_keep_calendar_report_and_timesheet_consistent() {
    let seeds: u64 = std::env::var("KUM_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let mut failures = Vec::new();
    for seed in 0..seeds {
        let bad = run_seed(seed);
        if !bad.is_empty() {
            failures.push(format!("tohum {seed}:\n  {}", bad.join("\n  ")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} / {seeds} gün bozuk:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Gerçek verinin bütün günleri (KUM_DB; isteğe bağlı KUM_ICS takvim dosyası).
#[test]
#[ignore = "gerçek veritabanı ister: KUM_DB=…/kum.db"]
fn real_database_is_consistent() {
    let source = std::env::var("KUM_DB").expect("KUM_DB");
    let copy = std::env::temp_dir().join(format!("kum-invariants-{}.db", Uuid::new_v4()));
    Store::copy_database(std::path::Path::new(&source), &copy).unwrap();
    let store = Store::open(&copy).unwrap();
    let calendar = std::env::var("KUM_ICS")
        .ok()
        .map(|p| Calendar::parse(&std::fs::read_to_string(p).unwrap()));
    let sessions = store
        .sessions_between(
            Utc.timestamp_opt(0, 0).unwrap(),
            Utc::now() + Duration::days(1),
        )
        .unwrap();
    let day = |t: DateTime<Utc>| t.with_timezone(&Local).date_naive();
    let (first, last) = (day(sessions.first().unwrap().started_at), day(Utc::now()));
    let mut failures = Vec::new();
    let mut date = first;
    while date <= last {
        let (from, to) = (midnight(date), midnight(date + Days::new(1)));
        let meetings = calendar
            .as_ref()
            .map(|c| c.meetings(from, to))
            .unwrap_or_default();
        let mut bad = check_day(&store, date, &meetings, false, STALE_SLACK);
        if date.weekday().num_days_from_monday() == 0 {
            bad.extend(check_span(&store, date, 7));
        }
        if !bad.is_empty() {
            failures.push(format!("{date}:\n  {}", bad.join("\n  ")));
        }
        date = date + Days::new(1);
    }
    drop(store);
    let _ = std::fs::remove_file(&copy);
    println!("{first} – {last} denetlendi");
    assert!(
        failures.is_empty(),
        "{} gün bozuk:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Tek bir rastgele gün (`SEED`), bozulan günü yinelemek için.
#[test]
#[ignore = "SEED=… ile tek gün"]
fn one_seed() {
    let seed: u64 = std::env::var("SEED").unwrap().parse().unwrap();
    let bad = run_seed(seed);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
