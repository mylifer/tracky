//! Takvimdeki çalışma blokları, molalar ve bağlam değişimi.
//!
//! - **Blok:** aynı kategoride kesintisiz çalışma. `BREAK_GAP`'ten uzun boşluk ya da en
//!   az `SWITCH_MIN` süren başka kategori yeni blok başlatır; daha kısa araya girmeler
//!   (bir mesaja bakmak gibi) bloğun içinde kalır. Aynı projeye atanmış iş kategorisi
//!   değişse de, arada `timesheet::MERGE_GAP`'i geçmeyen boşluk olsa da tek bloktur:
//!   zaman çizelgesindeki satır gibi.
//! - **Mola:** iki blok arasındaki `BREAK_GAP`–`LONG_GAP` arası boşluk (gece gibi
//!   daha uzun boşluklar mola sayılmaz).
//! - **Bağlam değişimi:** mola vermeden bir uygulamadan başka uygulamaya geçiş.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// Bu kadar boşluk mola sayılır ve bloğu böler.
pub const BREAK_GAP: Duration = Duration::minutes(5);
/// Başka kategoride bu kadar kalınırsa yeni blok başlar.
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
}

/// Bir takvim bloğundaki süre, bilgisayar başına.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDevice {
    pub id: String,
    pub name: String,
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
    // Bloğun kategorisinden farklı, henüz yeni blok sayılacak kadar uzamamış dilimler.
    let mut pending: Vec<&Activity> = Vec::new();
    let mut breaks = 0;

    for item in items.iter().copied() {
        let Some(b) = current.as_mut() else {
            current = Some(Builder::new(item));
            continue;
        };
        let last_end = pending.last().map_or(b.end, |p| p.end.max(b.end));
        let gap = item.start - last_end;
        // Bloğun projesine atanmış iş kategorisi farklı olsa da bloğu sürdürür.
        let same_project = item.project.is_some() && item.project == b.project.as_deref();
        if gap >= BREAK_GAP && !(same_project && gap <= crate::timesheet::MERGE_GAP) {
            for p in pending.drain(..) {
                b.add(p);
            }
            if gap < LONG_GAP {
                breaks += gap.num_seconds();
            }
            blocks.push(current.take().expect("blok var").finish());
            current = Some(Builder::new(item));
            continue;
        }
        if item.category == b.category.as_deref() || same_project {
            for p in pending.drain(..) {
                b.add(p);
            }
            b.add(item);
            continue;
        }
        pending.push(item);
        let away: Duration = pending.iter().map(|p| p.end - p.start).sum();
        if away >= SWITCH_MIN {
            blocks.push(current.take().expect("blok var").finish());
            let mut next = Builder::new(pending[0]);
            for p in &pending[1..] {
                next.add(p);
            }
            next.category = next.dominant();
            pending.clear();
            current = Some(next);
        }
    }
    if let Some(mut b) = current {
        for p in pending {
            b.add(p);
        }
        blocks.push(b.finish());
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

fn per_hour_x10(switches: u32, active_seconds: i64) -> u32 {
    if active_seconds <= 0 {
        return 0;
    }
    (f64::from(switches) * 36_000.0 / active_seconds as f64).round() as u32
}

struct Builder {
    /// Bloğun kategorisi (ilk dilimden; başka kategoriden açılırsa baskın olan).
    category: Option<String>,
    /// Bloğa en son eklenen projeli dilimin projesi.
    project: Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    active: i64,
    switches: u32,
    last_app: String,
    categories: HashMap<Option<String>, i64>,
    projects: HashMap<String, i64>,
    /// Uygulama adı -> (süre, ilk görülen uygulama kimliği).
    apps: HashMap<String, (i64, String)>,
}

impl Builder {
    fn new(item: &Activity) -> Self {
        let mut b = Self {
            category: item.category.map(str::to_string),
            project: None,
            start: item.start,
            end: item.start,
            active: 0,
            switches: 0,
            last_app: item.app_id.to_string(),
            categories: HashMap::new(),
            projects: HashMap::new(),
            apps: HashMap::new(),
        };
        b.add(item);
        b
    }

    fn add(&mut self, item: &Activity) {
        let secs = (item.end - item.start.max(self.end)).num_seconds().max(0);
        if item.app_id != self.last_app {
            self.switches += 1;
            self.last_app = item.app_id.to_string();
        }
        self.active += secs;
        self.end = self.end.max(item.end);
        *self
            .categories
            .entry(item.category.map(str::to_string))
            .or_default() += secs;
        if let Some(p) = item.project {
            self.project = Some(p.to_string());
            *self.projects.entry(p.to_string()).or_default() += secs;
        }
        self.apps
            .entry(item.app_name.to_string())
            .or_insert_with(|| (0, item.app_id.to_string()))
            .0 += secs;
    }

    fn dominant(&self) -> Option<String> {
        self.categories
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .and_then(|(c, _)| c.clone())
    }

    fn finish(self) -> WorkBlock {
        let category_id = self
            .categories
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .and_then(|(c, _)| c);
        let project_id = self
            .projects
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .filter(|(_, secs)| *secs * 2 >= self.active && *secs > 0)
            .map(|(p, _)| p);
        let mut apps: Vec<BlockApp> = self
            .apps
            .into_iter()
            .map(|(app_name, (seconds, app_id))| BlockApp {
                app_id,
                app_name,
                seconds,
            })
            .collect();
        apps.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));
        apps.truncate(3);
        WorkBlock {
            start: self.start,
            end: self.end,
            active_seconds: self.active,
            category_id,
            project_id,
            switches: self.switches,
            top_apps: apps,
            devices: Vec::new(),
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
        }
    }

    #[test]
    fn splits_blocks_on_category_runs_and_breaks() {
        let items = [
            a("Code", Some("dev"), 0, 30),
            a("Slack", Some("comm"), 30, 31), // kısa göz atma: blok içinde kalır
            a("Terminal", Some("dev"), 31, 50),
            a("Slack", Some("comm"), 50, 58), // 8 dk: yeni blok
            a("Safari", None, 70, 75),        // 12 dk mola: yeni blok
            a("Code", Some("dev"), 600, 610), // gece boşluğu: mola sayılmaz
        ];
        let s = analyze(&items);
        let cats: Vec<_> = s.blocks.iter().map(|b| b.category_id.as_deref()).collect();
        assert_eq!(cats, [Some("dev"), Some("comm"), None, Some("dev")]);
        let first = &s.blocks[0];
        assert_eq!(first.active_seconds, 50 * 60);
        assert_eq!(first.top_apps[0].app_name, "Code");
        assert_eq!(s.break_seconds, 12 * 60);
        // Code→Slack→Terminal→Slack ve Safari→(mola) geçişleri sayılmaz.
        assert_eq!(s.switches, 3);
    }

    #[test]
    fn block_project_needs_half_of_the_block() {
        let p = |mut x: Activity<'static>, project| {
            x.project = project;
            x
        };
        let items = [
            p(a("Code", Some("dev"), 0, 30), Some("kum")),
            p(a("Code", Some("dev"), 30, 50), None),
            p(a("Code", Some("dev"), 100, 110), Some("kum")),
            p(a("Code", Some("dev"), 110, 130), Some("x")),
            p(a("Code", Some("dev"), 130, 160), None),
        ];
        let s = analyze(&items);
        let projects: Vec<_> = s.blocks.iter().map(|b| b.project_id.as_deref()).collect();
        assert_eq!(projects, [Some("kum"), None]);
    }

    #[test]
    fn same_project_joins_categories_and_short_gaps() {
        let p = |mut x: Activity<'static>, project| {
            x.project = project;
            x
        };
        let items = [
            p(a("Code", Some("dev"), 0, 20), Some("kum")),
            p(a("Slack", Some("comm"), 20, 40), Some("kum")), // başka kategori, aynı proje
            p(a("Code", Some("dev"), 50, 60), Some("kum")),   // 10 dk boşluk: aynı blok
            p(a("Code", Some("dev"), 80, 90), Some("kum")),   // 20 dk boşluk: yeni blok
            p(a("Slack", Some("comm"), 90, 100), None),       // projesiz başka kategori: yeni blok
        ];
        let s = analyze(&items);
        let spans: Vec<_> = s
            .blocks
            .iter()
            .map(|b| (b.start, b.end, b.project_id.as_deref()))
            .collect();
        assert_eq!(
            spans,
            [
                (t(0), t(60), Some("kum")),
                (t(80), t(90), Some("kum")),
                (t(90), t(100), None),
            ]
        );
        assert_eq!(s.blocks[0].active_seconds, 50 * 60);
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
