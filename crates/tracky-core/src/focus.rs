//! Çalışma blokları, odak süresi, bağlam değişimi ve odak skoru.
//!
//! Kum'un kendi tanımları (Rize'ninki gibi kapalı değil, burada açık):
//! - **Blok (oturum):** aynı kategoride kesintisiz çalışma. `BREAK_GAP`'ten uzun
//!   boşluk ya da en az `SWITCH_MIN` süren başka kategori yeni blok başlatır; daha
//!   kısa araya girmeler (bir mesaja bakmak gibi) bloğun içinde kalır.
//! - **Odak bloğu:** en az `FOCUS_MIN` sürer ve süresinin en az `FOCUS_SHARE`'i tek
//!   kategoridedir.
//! - **Mola:** iki blok arasındaki `BREAK_GAP`–`LONG_GAP` arası boşluk (gece gibi
//!   daha uzun boşluklar mola sayılmaz).
//! - **Bağlam değişimi:** bir blok içinde bir uygulamadan başka uygulamaya geçiş.
//! - **Odak skoru (0–100):** odak süresinin payı (%55), saatlik bağlam değişiminin
//!   azlığı (%25) ve en uzun odak bloğunun uzunluğu (%20).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// Bu kadar boşluk mola sayılır ve bloğu böler.
pub const BREAK_GAP: Duration = Duration::minutes(5);
pub const FOCUS_MIN: Duration = Duration::minutes(20);
pub const FOCUS_SHARE: f64 = 0.75;
/// Başka kategoride bu kadar kalınırsa yeni blok başlar.
pub const SWITCH_MIN: Duration = Duration::minutes(3);
/// Bundan uzun boşluk mola değil, günün bitişidir.
pub const LONG_GAP: Duration = Duration::hours(3);
/// Saatte bu kadar değişime kadar tam puan, `SWITCH_WORST`'te sıfır.
const SWITCH_OK: f64 = 10.0;
const SWITCH_WORST: f64 = 60.0;
/// En uzun odak bloğu bu süreye ulaşınca tam puan.
const LONGEST_FULL: f64 = 90.0 * 60.0;

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
    pub focus: bool,
    pub switches: u32,
    /// En çok kullanılan uygulamalar (en fazla 3).
    pub top_apps: Vec<BlockApp>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusStats {
    pub score: u8,
    pub active_seconds: i64,
    pub focus_seconds: i64,
    pub break_seconds: i64,
    pub switches: u32,
    /// Etkin saat başına bağlam değişimi (×10, tamsayı serileştirme için).
    pub switches_per_hour_x10: u32,
    pub longest_focus_seconds: i64,
    pub blocks: Vec<WorkBlock>,
}

/// Etkinlikler başlangıca göre sıralı olmalı.
pub fn analyze(items: &[Activity]) -> FocusStats {
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
        if gap >= BREAK_GAP {
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
        if item.category == b.category.as_deref() {
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
    let focus: i64 = blocks
        .iter()
        .filter(|b| b.focus)
        .map(|b| b.active_seconds)
        .sum();
    let longest = blocks
        .iter()
        .filter(|b| b.focus)
        .map(|b| b.active_seconds)
        .max()
        .unwrap_or(0);
    let hours = active as f64 / 3600.0;
    let per_hour = if hours > 0.0 {
        switches as f64 / hours
    } else {
        0.0
    };

    FocusStats {
        score: score(active, focus, per_hour, longest),
        active_seconds: active,
        focus_seconds: focus,
        break_seconds: breaks,
        switches,
        switches_per_hour_x10: (per_hour * 10.0).round() as u32,
        longest_focus_seconds: longest,
        blocks,
    }
}

/// Günlük analizleri birleştirir (haftalık özet için): toplamlar toplanır,
/// skor etkin süreye göre ağırlıklı ortalamadır.
pub fn merge(days: Vec<FocusStats>) -> FocusStats {
    let active: i64 = days.iter().map(|d| d.active_seconds).sum();
    let weighted: i64 = days.iter().map(|d| d.score as i64 * d.active_seconds).sum();
    let switches: u32 = days.iter().map(|d| d.switches).sum();
    let hours = active as f64 / 3600.0;
    FocusStats {
        score: if active > 0 {
            (weighted / active) as u8
        } else {
            0
        },
        active_seconds: active,
        focus_seconds: days.iter().map(|d| d.focus_seconds).sum(),
        break_seconds: days.iter().map(|d| d.break_seconds).sum(),
        switches,
        switches_per_hour_x10: if hours > 0.0 {
            (switches as f64 / hours * 10.0).round() as u32
        } else {
            0
        },
        longest_focus_seconds: days
            .iter()
            .map(|d| d.longest_focus_seconds)
            .max()
            .unwrap_or(0),
        blocks: days.into_iter().flat_map(|d| d.blocks).collect(),
    }
}

fn score(active: i64, focus: i64, per_hour: f64, longest: i64) -> u8 {
    if active < 60 {
        return 0;
    }
    let focus_ratio = focus as f64 / active as f64;
    let calm = (1.0 - (per_hour - SWITCH_OK) / (SWITCH_WORST - SWITCH_OK)).clamp(0.0, 1.0);
    let depth = (longest as f64 / LONGEST_FULL).clamp(0.0, 1.0);
    (100.0 * (0.55 * focus_ratio + 0.25 * calm + 0.20 * depth)).round() as u8
}

struct Builder {
    /// Bloğun kategorisi (ilk dilimden; başka kategoriden açılırsa baskın olan).
    category: Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    active: i64,
    switches: u32,
    last_app: String,
    categories: HashMap<Option<String>, i64>,
    projects: HashMap<String, i64>,
    apps: HashMap<String, i64>,
}

impl Builder {
    fn new(item: &Activity) -> Self {
        let mut b = Self {
            category: item.category.map(str::to_string),
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
            *self.projects.entry(p.to_string()).or_default() += secs;
        }
        *self.apps.entry(item.app_name.to_string()).or_default() += secs;
    }

    fn dominant(&self) -> Option<String> {
        self.categories
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .and_then(|(c, _)| c.clone())
    }

    fn finish(self) -> WorkBlock {
        let (category_id, top) = self
            .categories
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .unwrap_or((None, 0));
        let project_id = self
            .projects
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .filter(|(_, secs)| *secs * 2 >= self.active && *secs > 0)
            .map(|(p, _)| p);
        let mut apps: Vec<BlockApp> = self
            .apps
            .into_iter()
            .map(|(app_name, seconds)| BlockApp { app_name, seconds })
            .collect();
        apps.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app_name.cmp(&b.app_name)));
        apps.truncate(3);
        let focus = Duration::seconds(self.active) >= FOCUS_MIN
            && top as f64 >= FOCUS_SHARE * self.active as f64;
        WorkBlock {
            start: self.start,
            end: self.end,
            active_seconds: self.active,
            category_id,
            project_id,
            focus,
            switches: self.switches,
            top_apps: apps,
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
        assert!(first.focus);
        assert_eq!(first.active_seconds, 50 * 60);
        assert_eq!(first.top_apps[0].app_name, "Code");
        assert!(!s.blocks[1].focus);
        assert_eq!(s.break_seconds, 12 * 60);
        assert_eq!(s.focus_seconds, 50 * 60);
        // Code→Slack→Terminal→Slack ve Safari→(mola) geçişleri sayılmaz.
        assert_eq!(s.switches, 3);
    }

    #[test]
    fn fragmented_work_scores_lower_than_deep_work() {
        let deep = [a("Code", Some("dev"), 0, 120)];
        let mut fragmented = Vec::new();
        let apps = ["Code", "Slack", "Safari", "Mail"];
        let cats = [Some("dev"), Some("comm"), None, Some("comm")];
        for i in 0..120 {
            fragmented.push(a(apps[i % 4], cats[i % 4], i as i64, i as i64 + 1));
        }
        let deep = analyze(&deep).score;
        let frag = analyze(&fragmented).score;
        assert_eq!(deep, 100);
        assert!(frag < 30, "dağınık iş skoru: {frag}");
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
    fn empty_day_scores_zero() {
        assert_eq!(analyze(&[]), FocusStats::default());
    }
}
