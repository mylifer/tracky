//! Takvimdeki çalışma blokları, molalar ve bağlam değişimi.
//!
//! - **Blok:** kesintisiz çalışma; yalnızca `BREAK_GAP`'ten uzun boşluk (mola) yeni blok
//!   başlatır. Uygulama ya da kategori değişse de blok sürer: blok uygulama akışından çıkar,
//!   takvimde en çok süren pencerenin adıyla görünür (Timely gibi; 18 Ekim'de kategori
//!   değiştikçe bölünen bloklar bir öğleden sonrayı 23 parçaya ayırıyordu). Bloğun projesine
//!   atanmış iş, arada `timesheet::MERGE_GAP`'i geçmeyen boşluk olsa da aynı bloktur: zaman
//!   çizelgesindeki satır gibi.
//! - **Projeye göre bölme:** bloğun projesinden (yarısını kaplayan; yoksa projesiz) başka
//!   projeye atanmış, en az `SWITCH_MIN` süren kısım kategorisi aynı olsa da ayrı bloktur
//!   ([`split_projects`]). O projenin kısımları arasındaki `MERGE_GAP`'i geçmeyen atanmamış
//!   süre o blokta kalır; bloğun projesindeki atanmamış süre bloğun kendisidir.
//! - **Elle bölme:** blok kısaltılınca kesilen kısım silinmez; o anda başlayan dilim
//!   ([`Activity::block_start`]) önceki bloğa katılmaz, yeni blok başlatır.
//! - **Mola:** iki blok arasındaki `BREAK_GAP`–`LONG_GAP` arası boşluk (gece gibi
//!   daha uzun boşluklar mola sayılmaz).
//! - **Bağlam değişimi:** mola vermeden bir uygulamadan başka uygulamaya geçiş.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// Bu kadar boşluk mola sayılır ve bloğu böler.
pub const BREAK_GAP: Duration = Duration::minutes(5);
/// Bloğun projesinden başka projeye atanmış kısım bu kadar sürerse ayrı blok olur.
pub const SWITCH_MIN: Duration = Duration::minutes(3);
/// Bundan uzun boşluk mola değil, günün bitişidir.
pub const LONG_GAP: Duration = Duration::hours(3);

/// Analize giren tek bir zaman dilimi (aralığa kırpılmış).
#[derive(Debug, Clone)]
pub struct Activity<'a> {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub app_id: &'a str,
    pub app_name: &'a str,
    pub category: Option<&'a str>,
    pub project: Option<&'a str>,
    /// Blok burada elle bölündü ([`crate::model::Session::block_from`]): yeni blok başlar.
    pub block_start: bool,
}

impl Activity<'_> {
    /// Oturumun `[start, end)` aralığına kırpılmış dilimi bloğu bölüyor mu: oturumun bölme
    /// anında başlayan parçası, kırpılmadan.
    pub fn starts_block(s: &crate::model::Session, start: DateTime<Utc>) -> bool {
        s.block_from == Some(s.started_at) && start == s.started_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockApp {
    /// Simgesi için: aynı adlı uygulamalardan ilk görüleninki.
    pub app_id: String,
    pub app_name: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBlock {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Bloğun içinde gerçekten etkin geçen süre (kısa boşluklar hariç).
    pub active_seconds: i64,
    /// En çok zaman geçen kategori.
    pub category_id: Option<String>,
    /// Bloğun en az yarısını kaplayan proje (yoksa `None`).
    pub project_id: Option<String>,
    pub switches: u32,
    /// En çok kullanılan uygulamalar (en fazla 3).
    pub top_apps: Vec<BlockApp>,
    /// Bloktaki süre bilgisayar başına (yalnızca aralıkta birden çok bilgisayar varsa dolu;
    /// `Store::report_for_device`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<BlockDevice>,
    /// Projesi bir zaman çizelgesine bağlıysa blok oraya gönderildi mi
    /// ([`crate::timesheet::Sent`]; `Store::build_report`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<bool>,
}

/// Bir takvim bloğundaki süre, bilgisayar başına.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDevice {
    pub id: String,
    pub name: String,
    pub os: String,
    pub model: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkStats {
    pub active_seconds: i64,
    pub break_seconds: i64,
    pub switches: u32,
    /// Etkin saat başına bağlam değişimi (×10, tamsayı serileştirme için).
    pub switches_per_hour_x10: u32,
    pub blocks: Vec<WorkBlock>,
}

/// Etkinlikler başlangıca göre sıralı olmalı.
pub fn analyze(items: &[Activity]) -> WorkStats {
    let items: Vec<&Activity> = items.iter().filter(|i| i.end > i.start).collect();
    let mut blocks: Vec<WorkBlock> = Vec::new();
    let mut current: Option<Builder> = None;
    let mut breaks = 0;

    for item in items.iter().copied() {
        let Some(b) = current.as_mut() else {
            current = Some(Builder::new(item));
            continue;
        };
        let gap = item.start - b.end;
        // Bloğun projesine atanmış iş `MERGE_GAP`'i geçmeyen boşluktan sonra da bloğu sürdürür.
        let same_project = item.project.is_some() && item.project == b.project.as_deref();
        let rest = gap >= BREAK_GAP && !(same_project && gap <= crate::timesheet::MERGE_GAP);
        if !item.block_start && !rest {
            b.add(item);
            continue;
        }
        if gap >= BREAK_GAP && gap < LONG_GAP {
            breaks += gap.num_seconds();
        }
        close(current.take().expect("blok var"), &mut blocks, &mut breaks);
        current = Some(Builder::new(item));
    }
    if let Some(b) = current {
        close(b, &mut blocks, &mut breaks);
    }

    // Bağlam değişimi: mola olmadan bir uygulamadan diğerine her geçiş.
    let switches = items
        .windows(2)
        .filter(|w| w[1].app_id != w[0].app_id && w[1].start - w[0].end < BREAK_GAP)
        .count() as u32;

    let active: i64 = blocks.iter().map(|b| b.active_seconds).sum();
    WorkStats {
        active_seconds: active,
        break_seconds: breaks,
        switches,
        switches_per_hour_x10: per_hour_x10(switches, active),
        blocks,
    }
}

/// Günlük analizleri birleştirir (hafta ve ay için): toplamlar toplanır.
pub fn merge(days: Vec<WorkStats>) -> WorkStats {
    let active: i64 = days.iter().map(|d| d.active_seconds).sum();
    let switches: u32 = days.iter().map(|d| d.switches).sum();
    WorkStats {
        active_seconds: active,
        break_seconds: days.iter().map(|d| d.break_seconds).sum(),
        switches,
        switches_per_hour_x10: per_hour_x10(switches, active),
        blocks: days.into_iter().flat_map(|d| d.blocks).collect(),
    }
}

/// Bloğu kapatır ([`Builder::finish`]); projeye göre bölünen parçalar arasındaki boşluk da
/// bloklar arası boşluk gibi mola sayılır.
fn close(b: Builder, blocks: &mut Vec<WorkBlock>, breaks: &mut i64) {
    let parts = b.finish();
    for w in parts.windows(2) {
        let gap = w[1].start - w[0].end;
        if gap >= BREAK_GAP && gap < LONG_GAP {
            *breaks += gap.num_seconds();
        }
    }
    blocks.extend(parts);
}

/// Bloğun dilimleri (sıralı), bloğun projesinden (`project`) başka projeye atanmış en az
/// `SWITCH_MIN` süren kısımlarından bölünmüş. Daha kısa araya girmeler bölmez. Aynı projenin
/// kısımları arasındaki atanmamış süre, `MERGE_GAP`'i geçmiyorsa ve kısmın en az yarısı yine
/// o projede kalıyorsa o kısımdadır (aradaki boşlukta olduğu gibi). Bölünecek bir şey yoksa
/// tek parça döner.
fn split_projects<'s>(
    items: &[&'s Activity<'s>],
    project: Option<&str>,
) -> Vec<Vec<&'s Activity<'s>>> {
    struct Run<'s> {
        project: Option<&'s str>,
        items: Vec<&'s Activity<'s>>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        /// Dilimlerin süresi (aradaki boşluklar hariç).
        length: Duration,
        /// Bunun `project`'e atanmış kısmı.
        owned: Duration,
    }
    impl<'s> Run<'s> {
        fn new(i: &'s Activity<'s>) -> Self {
            Self {
                project: i.project,
                items: vec![i],
                start: i.start,
                end: i.end,
                length: i.end - i.start,
                owned: i.end - i.start,
            }
        }
        fn absorb(&mut self, other: Run<'s>) {
            if other.project == self.project {
                self.owned += other.owned;
            }
            self.items.extend(other.items);
            self.end = self.end.max(other.end);
            self.length += other.length;
        }
        /// Kısmın ağırlığı: başka projenin kısmıysa o projede geçen süre.
        fn weight(&self) -> Duration {
            if self.project.is_some() {
                self.owned
            } else {
                self.length
            }
        }
    }

    // Aynı projeli ardışık dilimler.
    let mut runs: Vec<Run> = Vec::new();
    for &i in items {
        match runs.last_mut() {
            Some(r) if r.project == i.project => r.absorb(Run::new(i)),
            _ => runs.push(Run::new(i)),
        }
    }
    // Bloğun projesi dışında bir proje yoksa bölünecek bir şey yok.
    let foreign = |p: Option<&'s str>| p.filter(|p| Some(*p) != project);
    if runs.iter().all(|r| foreign(r.project).is_none()) {
        return vec![items.to_vec()];
    }

    // Aynı projeye dönülen araya girmeler o projenin kısmına katılır.
    let mut joined: Vec<Run> = Vec::new();
    for r in runs {
        if let [.., prev, mid] = joined.as_slice()
            && prev.project == r.project
            && (mid.length < SWITCH_MIN
                || (r.project.is_some()
                    && mid.project.is_none()
                    && r.start - prev.end <= crate::timesheet::MERGE_GAP
                    && (prev.owned + r.owned) * 2 >= prev.length + mid.length + r.length))
        {
            let mid = joined.pop().expect("araya giren var");
            let prev = joined.last_mut().expect("önceki var");
            prev.absorb(mid);
            prev.absorb(r);
            continue;
        }
        joined.push(r);
    }

    // Bloğun projesi ve atanmamış süre bir arada kalır; kalan kısa kısımlar komşusuna katılır.
    let mut parts: Vec<Run> = Vec::new();
    for mut r in joined {
        r.project = foreign(r.project);
        let first = parts.len() == 1;
        match parts.last_mut() {
            Some(p) if r.weight() < SWITCH_MIN || p.project == r.project => p.absorb(r),
            // Blok kısa bir kısımla başladıysa o kısım ardından gelene katılır.
            Some(p) if first && p.weight() < SWITCH_MIN => {
                let head = std::mem::replace(p, r);
                p.items.splice(0..0, head.items);
                p.start = head.start;
                p.length += head.length;
            }
            _ => parts.push(r),
        }
    }
    parts.into_iter().map(|p| p.items).collect()
}

fn per_hour_x10(switches: u32, active_seconds: i64) -> u32 {
    if active_seconds <= 0 {
        return 0;
    }
    (f64::from(switches) * 36_000.0 / active_seconds as f64).round() as u32
}

struct Builder<'s> {
    /// Bloğa en son eklenen projeli dilimin projesi.
    project: Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    /// Süreler milisaniye: binlerce kısa dilimin her biri saniyeye kırpılsa blok raporun
    /// toplamından dakikalarca kısa kalırdı. Saniyeye en sonda çevrilir.
    active: i64,
    switches: u32,
    last_app: String,
    categories: HashMap<Option<String>, i64>,
    projects: HashMap<String, i64>,
    /// Uygulama adı -> (süre, ilk görülen uygulama kimliği).
    apps: HashMap<String, (i64, String)>,
    /// Projeye göre bölmek için ([`split_projects`]).
    items: Vec<&'s Activity<'s>>,
}

impl<'s> Builder<'s> {
    fn new(item: &'s Activity<'s>) -> Self {
        let mut b = Self {
            project: None,
            start: item.start,
            end: item.start,
            active: 0,
            switches: 0,
            last_app: item.app_id.to_string(),
            categories: HashMap::new(),
            projects: HashMap::new(),
            apps: HashMap::new(),
            items: Vec::new(),
        };
        b.add(item);
        b
    }

    fn add(&mut self, item: &'s Activity<'s>) {
        let ms = (item.end - item.start.max(self.end))
            .num_milliseconds()
            .max(0);
        if item.app_id != self.last_app {
            self.switches += 1;
            self.last_app = item.app_id.to_string();
        }
        self.active += ms;
        self.end = self.end.max(item.end);
        *self
            .categories
            .entry(item.category.map(str::to_string))
            .or_default() += ms;
        if let Some(p) = item.project {
            self.project = Some(p.to_string());
            *self.projects.entry(p.to_string()).or_default() += ms;
        }
        self.apps
            .entry(item.app_name.to_string())
            .or_insert_with(|| (0, item.app_id.to_string()))
            .0 += ms;
        self.items.push(item);
    }

    /// Bloğun en az yarısını kaplayan proje.
    fn project_id(&self) -> Option<String> {
        self.projects
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .filter(|(_, ms)| **ms * 2 >= self.active && **ms > 0)
            .map(|(p, _)| p.clone())
    }

    /// Blok; başka projeye atanmış kısımları ayrı bloklar olur ([`split_projects`]).
    fn finish(self) -> Vec<WorkBlock> {
        let parts = split_projects(&self.items, self.project_id().as_deref());
        if parts.len() < 2 {
            return vec![self.block()];
        }
        parts
            .into_iter()
            .map(|part| {
                let mut b = Builder::new(part[0]);
                for i in &part[1..] {
                    b.add(i);
                }
                b.block()
            })
            .collect()
    }

    fn block(self) -> WorkBlock {
        let project_id = self.project_id();
        let category_id = self
            .categories
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .and_then(|(c, _)| c);
        let mut apps: Vec<(String, (i64, String))> = self.apps.into_iter().collect();
        apps.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        let apps: Vec<BlockApp> = apps
            .into_iter()
            .take(3)
            .map(|(app_name, (ms, app_id))| BlockApp {
                app_id,
                app_name,
                seconds: ms / 1000,
            })
            .collect();
        WorkBlock {
            start: self.start,
            end: self.end,
            active_seconds: self.active / 1000,
            category_id,
            project_id,
            switches: self.switches,
            top_apps: apps,
            devices: Vec::new(),
            sent: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::minutes(min)
    }

    fn a<'a>(app: &'a str, cat: Option<&'a str>, from: i64, to: i64) -> Activity<'a> {
        Activity {
            start: t(from),
            end: t(to),
            app_id: app,
            app_name: app,
            category: cat,
            project: None,
            block_start: false,
        }
    }

    #[test]
    fn splits_blocks_only_on_breaks() {
        let items = [
            a("Code", Some("dev"), 0, 30),
            a("Slack", Some("comm"), 30, 31), // kısa göz atma: blok içinde kalır
            a("Terminal", Some("dev"), 31, 50),
            a("Slack", Some("comm"), 50, 58), // başka kategoride 8 dk: blok sürer
            a("Safari", None, 70, 75),        // 12 dk mola: yeni blok
            a("Code", Some("dev"), 600, 610), // gece boşluğu: mola sayılmaz
        ];
        let s = analyze(&items);
        let cats: Vec<_> = s.blocks.iter().map(|b| b.category_id.as_deref()).collect();
        assert_eq!(cats, [Some("dev"), None, Some("dev")]);
        let first = &s.blocks[0];
        assert_eq!((first.start, first.end), (t(0), t(58)));
        assert_eq!(first.active_seconds, 58 * 60);
        assert_eq!(first.top_apps[0].app_name, "Code");
        assert_eq!(s.break_seconds, 12 * 60);
        // Code→Slack→Terminal→Slack ve Safari→(mola) geçişleri sayılmaz.
        assert_eq!(s.switches, 3);
    }

    #[test]
    fn manual_split_starts_a_new_block() {
        let mut second = a("Code", Some("dev"), 30, 60);
        second.block_start = true;
        let s = analyze(&[a("Code", Some("dev"), 0, 30), second]);
        let spans: Vec<_> = s.blocks.iter().map(|b| (b.start, b.end)).collect();
        assert_eq!(spans, [(t(0), t(30)), (t(30), t(60))]);
        assert_eq!(s.break_seconds, 0);
    }

    fn p(mut x: Activity<'static>, project: Option<&'static str>) -> Activity<'static> {
        x.project = project;
        x
    }

    fn spans(s: &WorkStats) -> Vec<(DateTime<Utc>, DateTime<Utc>, Option<&str>)> {
        s.blocks
            .iter()
            .map(|b| (b.start, b.end, b.project_id.as_deref()))
            .collect()
    }

    #[test]
    fn block_project_needs_half_of_the_block() {
        let items = [
            p(a("Code", Some("dev"), 0, 30), Some("kum")),
            p(a("Code", Some("dev"), 30, 50), None),
            // Kısa kısımlar bölmez; blokta yarıya ulaşmayan proje bloğun projesi değildir.
            p(a("Code", Some("dev"), 100, 102), Some("kum")),
            p(a("Code", Some("dev"), 102, 110), None),
            p(a("Code", Some("dev"), 110, 112), Some("kum")),
            p(a("Code", Some("dev"), 112, 120), None),
        ];
        let s = analyze(&items);
        let projects: Vec<_> = s.blocks.iter().map(|b| b.project_id.as_deref()).collect();
        assert_eq!(projects, [Some("kum"), None]);
    }

    #[test]
    fn assigned_part_leaves_the_block() {
        // Projesiz bir gecenin ortasında projeye atanmış oyun (kategorisi aynı).
        let items = [
            a("Firefox", None, 0, 30),
            p(a("Game", None, 30, 60), Some("lol")),
            a("Firefox", None, 60, 62), // kısa araya girme
            p(a("Game", None, 62, 80), Some("lol")),
            a("Launcher", None, 80, 85), // projenin kısımları arasında atanmamış süre
            p(a("Game", None, 85, 90), Some("lol")),
            a("Firefox", None, 90, 150),
        ];
        let s = analyze(&items);
        assert_eq!(
            spans(&s),
            [
                (t(0), t(30), None),
                (t(30), t(90), Some("lol")),
                (t(90), t(150), None),
            ]
        );
        assert_eq!(s.blocks[1].active_seconds, 60 * 60);
        assert_eq!(s.break_seconds, 0);
    }

    #[test]
    fn other_project_leaves_a_project_block() {
        let items = [
            p(a("Code", Some("dev"), 0, 40), Some("kum")),
            a("Code", Some("dev"), 40, 55), // bloğun projesinde atanmamış süre: blokta kalır
            p(a("Code", Some("dev"), 55, 100), Some("kum")),
            p(a("Code", Some("dev"), 100, 102), Some("x")), // kısa: bölmez
            p(a("Code", Some("dev"), 102, 140), Some("kum")),
            p(a("Code", Some("dev"), 140, 160), Some("x")),
            p(a("Code", Some("dev"), 160, 200), Some("kum")),
        ];
        let s = analyze(&items);
        assert_eq!(
            spans(&s),
            [
                (t(0), t(140), Some("kum")),
                (t(140), t(160), Some("x")),
                (t(160), t(200), Some("kum")),
            ]
        );
    }

    #[test]
    fn mostly_unassigned_run_does_not_leave_as_project() {
        // Arada uzun atanmamış süre: proje kısmına katılırsa kısmın çoğu projesiz olurdu.
        let items = [
            a("Firefox", None, 0, 30),
            p(a("Game", None, 30, 34), Some("lol")),
            a("Firefox", None, 34, 46),
            p(a("Game", None, 46, 50), Some("lol")),
            a("Firefox", None, 50, 80),
        ];
        let s = analyze(&items);
        assert_eq!(
            spans(&s),
            [
                (t(0), t(30), None),
                (t(30), t(34), Some("lol")),
                (t(34), t(46), None),
                (t(46), t(50), Some("lol")),
                (t(50), t(80), None),
            ]
        );
    }

    #[test]
    fn same_project_joins_categories_and_short_gaps() {
        let items = [
            p(a("Code", Some("dev"), 0, 20), Some("kum")),
            p(a("Slack", Some("comm"), 20, 40), Some("kum")), // başka kategori, aynı proje
            p(a("Code", Some("dev"), 50, 60), Some("kum")),   // 10 dk boşluk: aynı blok
            p(a("Code", Some("dev"), 80, 90), Some("kum")),   // 20 dk boşluk: yeni blok
            p(a("Slack", Some("comm"), 90, 100), None),       // projesiz, molasız: blokta kalır
        ];
        let s = analyze(&items);
        assert_eq!(
            spans(&s),
            [(t(0), t(60), Some("kum")), (t(80), t(100), Some("kum"))]
        );
        assert_eq!(s.blocks[0].active_seconds, 50 * 60);
    }

    #[test]
    fn short_slices_add_up_in_milliseconds() {
        // 1,5 sn'lik 1200 dilim = 30 dk; saniyeye tek tek kırpılsa 20 dk kalırdı.
        let names: Vec<String> = (0..1200).map(|i| format!("app{}", i % 2)).collect();
        let items: Vec<Activity> = names
            .iter()
            .enumerate()
            .map(|(i, name)| Activity {
                start: t(0) + Duration::milliseconds(i as i64 * 1500),
                end: t(0) + Duration::milliseconds(i as i64 * 1500 + 1500),
                app_id: name,
                app_name: name,
                category: Some("dev"),
                project: None,
                block_start: false,
            })
            .collect();
        let s = analyze(&items);
        assert_eq!(s.blocks.len(), 1);
        assert_eq!(s.blocks[0].active_seconds, 30 * 60);
        assert_eq!(s.blocks[0].top_apps[0].seconds, 15 * 60);
    }

    #[test]
    fn empty_day_is_empty() {
        assert_eq!(analyze(&[]), WorkStats::default());
    }

    #[test]
    fn switches_per_hour() {
        assert_eq!(per_hour_x10(15, 3600), 150);
        assert_eq!(per_hour_x10(1, 0), 0);
    }
}
