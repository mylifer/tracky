//! Takvim (iCalendar / .ics): Outlook'un yayımladığı takvimden toplantılar.
//!
//! Dosya bir kez ayrıştırılır ([`Calendar::parse`]), istenen aralıktaki toplantılar
//! [`Calendar::meetings`] ile çıkarılır. Tekrarlayan toplantılar (RRULE: günlük, haftalık,
//! aylık, yıllık; INTERVAL, COUNT, UNTIL, BYDAY, BYMONTHDAY, BYMONTH, BYSETPOS), atlanan
//! günler (EXDATE) ve tek seferlik değişiklikler (RECURRENCE-ID) desteklenir.
//!
//! Saatler toplantının kendi saat diliminde açılır, sonra UTC'ye çevrilir. Outlook saat
//! dilimini Windows adıyla ("Turkey Standard Time") yazar ve tanımını (VTIMEZONE) dosyaya
//! ekler; bu tanım kullanılır. Tanımı olmayan dilim yerel saat sayılır.
//!
//! Tüm gün etkinlikleri, iptal edilenler ve "boş" (FREE, TRANSPARENT) ya da "ofis dışı"
//! görünenler toplantı sayılmaz.

use std::collections::HashMap;

use chrono::{
    DateTime, Datelike, Days, Duration, Local, Months, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Utc, Weekday,
};

use crate::timesheet::Meeting;

/// Bir tekrar kuralını açarken bakılan en çok dönem (sonsuz kurala karşı).
const MAX_PERIODS: usize = 20_000;
/// Bundan uzun etkinlik (çok günlük blok) toplantı sayılmaz.
const MAX_LENGTH: Duration = Duration::hours(12);

/// Konusu bunlarla başlayan etkinlik iptal edilmiştir (Outlook iptali böyle de yazar).
const CANCELLED_PREFIXES: &[&str] = &["canceled:", "cancelled:", "iptal edildi:", "iptal:"];
/// Konum ya da açıklamada bunlardan biri geçen toplantı çevrim içidir.
const ONLINE_MARKERS: &[&str] = &[
    "teams.microsoft.com",
    "microsoft teams",
    "zoom.us",
    "meet.google.com",
    "webex.com",
    "gotomeeting",
];

/// Ayrıştırılmış takvim.
#[derive(Debug, Clone, Default)]
pub struct Calendar {
    events: Vec<Event>,
    zones: HashMap<String, Zone>,
}

/// Saati bilinen bir an: UTC, belirli bir saat dilimi ya da (dilimsiz) yerel.
#[derive(Debug, Clone, PartialEq)]
enum Stamp {
    Utc(NaiveDateTime),
    Zoned(NaiveDateTime, String),
    Floating(NaiveDateTime),
}

impl Stamp {
    fn naive(&self) -> NaiveDateTime {
        match self {
            Stamp::Utc(n) | Stamp::Zoned(n, _) | Stamp::Floating(n) => *n,
        }
    }

    /// Aynı saat diliminde başka bir an.
    fn with_naive(&self, n: NaiveDateTime) -> Stamp {
        match self {
            Stamp::Utc(_) => Stamp::Utc(n),
            Stamp::Zoned(_, tz) => Stamp::Zoned(n, tz.clone()),
            Stamp::Floating(_) => Stamp::Floating(n),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Event {
    uid: String,
    summary: String,
    location: String,
    online: bool,
    start: Option<Stamp>,
    end: Option<Stamp>,
    duration: Option<Duration>,
    all_day: bool,
    skip: bool,
    rrule: Option<String>,
    exdates: Vec<Stamp>,
    recurrence_id: Option<Stamp>,
}

/// VTIMEZONE: standart ve yaz saati geçişleri.
#[derive(Debug, Clone, Default)]
struct Zone {
    observances: Vec<Observance>,
}

#[derive(Debug, Clone)]
struct Observance {
    /// İlk geçiş (yerel, geçişten önceki saatle).
    start: NaiveDateTime,
    offset_from: i32,
    offset_to: i32,
    /// Yıllık tekrar: (ay, haftanın günü, kaçıncı; -1 son) ya da (ay, ayın günü).
    yearly: Option<Yearly>,
}

#[derive(Debug, Clone, Copy)]
enum Yearly {
    Weekday(u32, Weekday, i32),
    MonthDay(u32, u32),
}

/// Bir satır: ad, parametreler ve değer (kaçış karakterleri açılmış).
struct Line<'a> {
    name: String,
    params: Vec<(String, &'a str)>,
    value: &'a str,
}

impl Line<'_> {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim_matches('"'))
    }
}

/// Katlanmış satırları (boşlukla başlayan devam satırları) birleştirir.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
        } else if !raw.is_empty() {
            out.push(raw.to_string());
        }
    }
    out
}

fn parse_line(line: &str) -> Option<Line<'_>> {
    // Değer, tırnak dışındaki ilk ':'den sonra başlar.
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v))
        })
        .collect();
    Some(Line {
        name,
        params,
        value,
    })
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_naive(v: &str) -> Option<NaiveDateTime> {
    let v = v.trim();
    if v.len() >= 15 {
        NaiveDateTime::parse_from_str(&v[..15], "%Y%m%dT%H%M%S").ok()
    } else {
        NaiveDate::parse_from_str(v, "%Y%m%d")
            .ok()
            .map(|d| d.and_time(NaiveTime::MIN))
    }
}

fn parse_stamp(value: &str, tzid: Option<&str>) -> Option<Stamp> {
    let value = value.trim();
    let naive = parse_naive(value)?;
    Some(if value.ends_with('Z') {
        Stamp::Utc(naive)
    } else if let Some(tz) = tzid {
        Stamp::Zoned(naive, tz.to_string())
    } else {
        Stamp::Floating(naive)
    })
}

/// "+0300" / "-0430" → saniye.
fn parse_offset(v: &str) -> Option<i32> {
    let v = v.trim();
    let (sign, digits) = match v.as_bytes().first()? {
        b'-' => (-1, &v[1..]),
        b'+' => (1, &v[1..]),
        _ => (1, v),
    };
    if digits.len() < 4 {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

/// "PT1H30M", "P1D", "-PT15M" → süre.
fn parse_duration(v: &str) -> Option<Duration> {
    let v = v.trim();
    let (neg, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    for c in v.chars() {
        match c {
            'T' => {}
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match unit {
                    'W' => Duration::weeks(n),
                    'D' => Duration::days(n),
                    'H' => Duration::hours(n),
                    'M' => Duration::minutes(n),
                    'S' => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

fn parse_weekday(v: &str) -> Option<Weekday> {
    Some(match v {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// "2TU", "-1SU", "MO" → (kaçıncı, gün); kaçıncı yoksa 0.
fn parse_byday(v: &str) -> Option<(i32, Weekday)> {
    let v = v.trim();
    let split = v.len().checked_sub(2)?;
    let (n, day) = v.split_at(split);
    let n = if n.is_empty() {
        0
    } else {
        n.trim_start_matches('+').parse().ok()?
    };
    Some((n, parse_weekday(day)?))
}

impl Calendar {
    /// .ics metnini ayrıştırır; tanınmayan satırlar atlanır.
    pub fn parse(text: &str) -> Self {
        let mut cal = Calendar::default();
        let mut event: Option<Event> = None;
        // (TZID, bölge) ve süren geçiş tanımı.
        let mut zone: Option<(String, Zone)> = None;
        let mut obs: Option<(NaiveDateTime, i32, i32, Option<Yearly>)> = None;
        for raw in unfold(text) {
            let Some(line) = parse_line(&raw) else {
                continue;
            };
            let value = line.value;
            match (
                line.name.as_str(),
                value.trim().to_ascii_uppercase().as_str(),
            ) {
                ("BEGIN", "VEVENT") => event = Some(Event::default()),
                ("END", "VEVENT") => {
                    if let Some(e) = event.take()
                        && e.start.is_some()
                    {
                        cal.events.push(e);
                    }
                }
                ("BEGIN", "VTIMEZONE") => zone = Some((String::new(), Zone::default())),
                ("END", "VTIMEZONE") => {
                    if let Some((id, z)) = zone.take()
                        && !id.is_empty()
                    {
                        cal.zones.insert(id, z);
                    }
                }
                ("BEGIN", "STANDARD" | "DAYLIGHT") if zone.is_some() => {
                    obs = Some((NaiveDateTime::MIN, 0, 0, None));
                }
                ("END", "STANDARD" | "DAYLIGHT") => {
                    if let (Some((start, from, to, yearly)), Some((_, z))) = (obs.take(), &mut zone)
                    {
                        z.observances.push(Observance {
                            start,
                            offset_from: from,
                            offset_to: to,
                            yearly,
                        });
                    }
                }
                _ => {
                    if let Some(o) = &mut obs {
                        match line.name.as_str() {
                            "DTSTART" => o.0 = parse_naive(value).unwrap_or(o.0),
                            "TZOFFSETFROM" => o.1 = parse_offset(value).unwrap_or(o.1),
                            "TZOFFSETTO" => o.2 = parse_offset(value).unwrap_or(o.2),
                            "RRULE" => o.3 = parse_yearly(value),
                            _ => {}
                        }
                    } else if let Some((id, _)) = &mut zone {
                        if line.name == "TZID" {
                            *id = value.trim().to_string();
                        }
                    } else if let Some(e) = &mut event {
                        e.apply(&line);
                    }
                }
            }
        }
        cal
    }

    /// Etkinlik sayısı (tekrarlar açılmadan).
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// `[from, to)` ile kesişen toplantılar, başlangıca göre sıralı.
    pub fn meetings(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Meeting> {
        // Tek seferlik değişiklikler: (UID, asıl başlangıç) → değişen etkinlik.
        let mut overrides: HashMap<(&str, DateTime<Utc>), &Event> = HashMap::new();
        for e in self.events.iter().filter(|e| e.recurrence_id.is_some()) {
            if let Some(at) = e.recurrence_id.as_ref().and_then(|s| self.to_utc(s)) {
                overrides.insert((e.uid.as_str(), at), e);
            }
        }
        let mut out = Vec::new();
        let mut push = |e: &Event, start: DateTime<Utc>, end: DateTime<Utc>| {
            let ok = !e.skip && !e.all_day && end > start && end - start <= MAX_LENGTH;
            if ok && start < to && end > from {
                out.push(Meeting {
                    uid: e.uid.clone(),
                    start,
                    end,
                    subject: e.summary.trim().to_string(),
                    location: e.location.trim().to_string(),
                    online: e.online,
                });
            }
        };
        for e in &self.events {
            let Some((start, length)) = self.span(e) else {
                continue;
            };
            if e.recurrence_id.is_some() {
                if let Some(s) = self.to_utc(&start) {
                    push(e, s, s + length);
                }
                continue;
            }
            let Some(rule) = e.rrule.as_deref() else {
                if let Some(s) = self.to_utc(&start) {
                    push(e, s, s + length);
                }
                continue;
            };
            let excluded: Vec<DateTime<Utc>> =
                e.exdates.iter().filter_map(|x| self.to_utc(x)).collect();
            // Açma, etkinliğin kendi dilimindeki yerel saatlerle yapılır (yaz saatinde de
            // toplantı aynı saatte kalır). Pencere bir gün genişletilir (dilim farkı).
            let until_local = |u: DateTime<Utc>| self.to_local_of(&start, u);
            let window_end = self.to_local_of(&start, to) + Duration::days(1);
            for occ in expand(rule, start.naive(), window_end, until_local) {
                let Some(s) = self.to_utc(&start.with_naive(occ)) else {
                    continue;
                };
                if excluded.contains(&s) {
                    continue;
                }
                match overrides.get(&(e.uid.as_str(), s)) {
                    Some(_) => {} // değişen hâli ayrıca eklenir
                    None => push(e, s, s + length),
                }
            }
        }
        out.sort_by(|a, b| a.start.cmp(&b.start).then(a.subject.cmp(&b.subject)));
        out
    }

    /// Başlangıç ve süre.
    fn span(&self, e: &Event) -> Option<(Stamp, Duration)> {
        let start = e.start.clone()?;
        let length = match (&e.end, e.duration) {
            (Some(end), _) => self.to_utc(end)? - self.to_utc(&start)?,
            (None, Some(d)) => d,
            (None, None) => Duration::zero(),
        };
        Some((start, length))
    }

    fn to_utc(&self, s: &Stamp) -> Option<DateTime<Utc>> {
        match s {
            Stamp::Utc(n) => Some(n.and_utc()),
            Stamp::Floating(n) => local_to_utc(*n),
            Stamp::Zoned(n, tz) => match self.zones.get(tz) {
                Some(zone) => Some((*n - Duration::seconds(zone.offset_at(*n).into())).and_utc()),
                None => local_to_utc(*n),
            },
        }
    }

    /// UTC anı, `like` damgasının saat dilimindeki yerel saate çevirir.
    fn to_local_of(&self, like: &Stamp, at: DateTime<Utc>) -> NaiveDateTime {
        match like {
            Stamp::Utc(_) => at.naive_utc(),
            Stamp::Floating(_) => at.with_timezone(&Local).naive_local(),
            Stamp::Zoned(_, tz) => match self.zones.get(tz) {
                Some(zone) => {
                    // Yerel saate göre ofset; geçiş anlarında bir saatlik sapma önemsizdir.
                    let guess =
                        at.naive_utc() + Duration::seconds(zone.offset_at(at.naive_utc()).into());
                    at.naive_utc() + Duration::seconds(zone.offset_at(guess).into())
                }
                None => at.with_timezone(&Local).naive_local(),
            },
        }
    }
}

fn local_to_utc(n: NaiveDateTime) -> Option<DateTime<Utc>> {
    Local
        .from_local_datetime(&n)
        .earliest()
        .or_else(|| {
            Local
                .from_local_datetime(&(n + Duration::hours(1)))
                .earliest()
        })
        .map(|d| d.with_timezone(&Utc))
}

fn parse_yearly(rule: &str) -> Option<Yearly> {
    let parts = rule_parts(rule);
    let month: u32 = parts.get("BYMONTH")?.parse().ok()?;
    if let Some(day) = parts.get("BYDAY") {
        let (n, wd) = parse_byday(day)?;
        Some(Yearly::Weekday(month, wd, if n == 0 { 1 } else { n }))
    } else {
        let d: u32 = parts.get("BYMONTHDAY")?.parse().ok()?;
        Some(Yearly::MonthDay(month, d))
    }
}

impl Zone {
    /// Yerel saatteki ofset (saniye): o ana kadarki son geçişin hedef ofseti.
    fn offset_at(&self, local: NaiveDateTime) -> i32 {
        let mut best: Option<(NaiveDateTime, i32)> = None;
        for o in &self.observances {
            for year in [local.year() - 1, local.year()] {
                let onset = match o.yearly {
                    Some(y) => match y.date_in(year) {
                        Some(d) if d.and_time(o.start.time()) >= o.start => {
                            d.and_time(o.start.time())
                        }
                        _ => continue,
                    },
                    None if year == local.year() => o.start,
                    None => continue,
                };
                if onset <= local && best.is_none_or(|(b, _)| onset > b) {
                    best = Some((onset, o.offset_to));
                }
            }
        }
        if let Some((_, off)) = best {
            return off;
        }
        // Hiçbir geçiş henüz olmadıysa en eski tanımın başlangıç ofseti.
        self.observances
            .iter()
            .min_by_key(|o| o.start)
            .map_or(0, |o| o.offset_from)
    }
}

impl Yearly {
    fn date_in(self, year: i32) -> Option<NaiveDate> {
        match self {
            Yearly::MonthDay(m, d) => NaiveDate::from_ymd_opt(year, m, d),
            Yearly::Weekday(m, wd, n) => nth_weekday(year, m, wd, n),
        }
    }
}

/// Ayın `n`. `wd` günü (`n` < 0: sondan).
fn nth_weekday(year: i32, month: u32, wd: Weekday, n: i32) -> Option<NaiveDate> {
    if n > 0 {
        NaiveDate::from_weekday_of_month_opt(year, month, wd, n as u8)
    } else {
        let days = month_days(year, month)?;
        let mut d = NaiveDate::from_ymd_opt(year, month, days)?;
        let mut seen = 0;
        loop {
            if d.weekday() == wd {
                seen -= 1;
                if seen == n {
                    return Some(d);
                }
            }
            if d.day() == 1 {
                return None;
            }
            d = d.pred_opt()?;
        }
    }
}

fn month_days(year: i32, month: u32) -> Option<u32> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = first.checked_add_months(Months::new(1))?;
    Some((next - first).num_days() as u32)
}

impl Event {
    fn apply(&mut self, line: &Line) {
        let value = line.value;
        let tzid = line.param("TZID");
        match line.name.as_str() {
            "UID" => self.uid = value.trim().to_string(),
            "SUMMARY" => {
                self.summary = unescape(value);
                // "İ" küçültülünce "i̇" olur (birleşen nokta): atılır.
                let lower = self.summary.trim().to_lowercase().replace('\u{307}', "");
                if CANCELLED_PREFIXES.iter().any(|p| lower.starts_with(p)) {
                    self.skip = true;
                }
            }
            "LOCATION" => {
                self.location = unescape(value);
                self.mark_online(&self.location.clone());
            }
            "DESCRIPTION" | "URL" | "X-ALT-DESC" => self.mark_online(value),
            "X-MICROSOFT-SKYPETEAMSMEETINGURL"
            | "X-MICROSOFT-ONLINEMEETINGCONFLINK"
            | "X-GOOGLE-CONFERENCE" => self.online |= !value.trim().is_empty(),
            "DTSTART" => {
                self.all_day = line.param("VALUE") == Some("DATE") || value.trim().len() == 8;
                self.start = parse_stamp(value, tzid);
            }
            "DTEND" => self.end = parse_stamp(value, tzid),
            "DURATION" => self.duration = parse_duration(value),
            "RRULE" => self.rrule = Some(value.trim().to_string()),
            "EXDATE" => {
                for v in value.split(',') {
                    if let Some(s) = parse_stamp(v, tzid) {
                        self.exdates.push(s);
                    }
                }
            }
            "RECURRENCE-ID" => self.recurrence_id = parse_stamp(value, tzid),
            "STATUS" => self.skip |= value.trim().eq_ignore_ascii_case("CANCELLED"),
            "TRANSP" => self.skip |= value.trim().eq_ignore_ascii_case("TRANSPARENT"),
            "X-MICROSOFT-CDO-BUSYSTATUS" => {
                let v = value.trim().to_ascii_uppercase();
                self.skip |= v == "FREE" || v == "OOF";
            }
            _ => {}
        }
    }

    fn mark_online(&mut self, text: &str) {
        let lower = text.to_lowercase();
        self.online |= ONLINE_MARKERS.iter().any(|m| lower.contains(m));
    }
}

fn rule_parts(rule: &str) -> HashMap<String, String> {
    rule.trim()
        .trim_start_matches("RRULE:")
        .split(';')
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v.trim().to_ascii_uppercase()))
        })
        .collect()
}

/// Tekrar kuralının `start`'tan `window_end`'e kadarki başlangıçları (yerel, sıralı).
/// `until_local` UTC'deki UNTIL değerini etkinliğin yerel saatine çevirir.
fn expand(
    rule: &str,
    start: NaiveDateTime,
    window_end: NaiveDateTime,
    until_local: impl Fn(DateTime<Utc>) -> NaiveDateTime,
) -> Vec<NaiveDateTime> {
    let p = rule_parts(rule);
    let freq = p.get("FREQ").map(String::as_str).unwrap_or("");
    let interval: u32 = p
        .get("INTERVAL")
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1);
    let count: Option<usize> = p.get("COUNT").and_then(|v| v.parse().ok());
    let until: Option<NaiveDateTime> = p.get("UNTIL").and_then(|v| {
        let naive = parse_naive(v)?;
        Some(if v.ends_with('Z') {
            until_local(naive.and_utc())
        } else if v.len() == 8 {
            // Yalnızca tarih: o günün sonuna kadar.
            naive + Duration::days(1) - Duration::seconds(1)
        } else {
            naive
        })
    });
    let list = |k: &str| -> Vec<String> {
        p.get(k)
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default()
    };
    let byday: Vec<(i32, Weekday)> = list("BYDAY")
        .iter()
        .filter_map(|d| parse_byday(d))
        .collect();
    let bymonthday: Vec<i32> = list("BYMONTHDAY")
        .iter()
        .filter_map(|d| d.parse().ok())
        .collect();
    let bymonth: Vec<u32> = list("BYMONTH")
        .iter()
        .filter_map(|d| d.parse().ok())
        .collect();
    let bysetpos: Vec<i32> = list("BYSETPOS")
        .iter()
        .filter_map(|d| d.parse().ok())
        .collect();
    let wkst = p
        .get("WKST")
        .and_then(|w| parse_weekday(w))
        .unwrap_or(Weekday::Mon);
    let time = start.time();
    let first = start.date();

    // Aydaki günlerin BYMONTHDAY / BYDAY süzgeci; ikisi de yoksa `default_day`.
    let month_candidates = |year: i32, month: u32, default_day: u32| -> Vec<NaiveDate> {
        let Some(days) = month_days(year, month) else {
            return Vec::new();
        };
        let mut out: Vec<NaiveDate> = if !bymonthday.is_empty() {
            bymonthday
                .iter()
                .filter_map(|&d| {
                    let d = if d < 0 { days as i32 + d + 1 } else { d };
                    (d >= 1).then(|| NaiveDate::from_ymd_opt(year, month, d as u32))?
                })
                .collect()
        } else if !byday.is_empty() {
            let mut v = Vec::new();
            for &(n, wd) in &byday {
                if n == 0 {
                    let mut d = NaiveDate::from_weekday_of_month_opt(year, month, wd, 1);
                    while let Some(x) = d.filter(|x| x.month() == month) {
                        v.push(x);
                        d = x.checked_add_days(Days::new(7));
                    }
                } else if let Some(x) = nth_weekday(year, month, wd, n) {
                    v.push(x);
                }
            }
            v
        } else {
            NaiveDate::from_ymd_opt(year, month, default_day)
                .into_iter()
                .collect()
        };
        // BYMONTHDAY ile BYDAY birlikteyse ikisine de uyan günler.
        if !bymonthday.is_empty() && !byday.is_empty() {
            out.retain(|d| byday.iter().any(|&(_, wd)| d.weekday() == wd));
        }
        out.sort();
        out.dedup();
        out
    };
    let setpos = |mut v: Vec<NaiveDate>| -> Vec<NaiveDate> {
        if bysetpos.is_empty() {
            return v;
        }
        v.sort();
        let n = v.len() as i32;
        let mut picked: Vec<NaiveDate> = bysetpos
            .iter()
            .filter_map(|&pos| {
                let i = if pos > 0 { pos - 1 } else { n + pos };
                (0..n).contains(&i).then(|| v[i as usize])
            })
            .collect();
        picked.sort();
        picked.dedup();
        picked
    };

    let mut out = Vec::new();
    let mut emitted = 0usize;
    for k in 0..MAX_PERIODS {
        let step = k as u32 * interval;
        let dates: Vec<NaiveDate> = match freq {
            "DAILY" => {
                let Some(d) = first.checked_add_days(Days::new(step.into())) else {
                    break;
                };
                let ok = (byday.is_empty() || byday.iter().any(|&(_, wd)| wd == d.weekday()))
                    && (bymonth.is_empty() || bymonth.contains(&d.month()))
                    && (bymonthday.is_empty() || bymonthday.contains(&(d.day() as i32)));
                if ok { vec![d] } else { vec![] }
            }
            "WEEKLY" => {
                let back =
                    (7 + first.weekday().num_days_from_monday() - wkst.num_days_from_monday()) % 7;
                let Some(week) = first
                    .checked_sub_days(Days::new(back.into()))
                    .and_then(|w| w.checked_add_days(Days::new(u64::from(step) * 7)))
                else {
                    break;
                };
                let days: Vec<Weekday> = if byday.is_empty() {
                    vec![first.weekday()]
                } else {
                    byday.iter().map(|&(_, wd)| wd).collect()
                };
                let v = (0..7)
                    .filter_map(|i| week.checked_add_days(Days::new(i)))
                    .filter(|d| days.contains(&d.weekday()))
                    .filter(|d| bymonth.is_empty() || bymonth.contains(&d.month()))
                    .collect();
                setpos(v)
            }
            "MONTHLY" => {
                let Some(m) = NaiveDate::from_ymd_opt(first.year(), first.month(), 1)
                    .and_then(|d| d.checked_add_months(Months::new(step)))
                else {
                    break;
                };
                if !bymonth.is_empty() && !bymonth.contains(&m.month()) {
                    continue;
                }
                setpos(month_candidates(m.year(), m.month(), first.day()))
            }
            "YEARLY" => {
                let year = first.year() + step as i32;
                let months = if bymonth.is_empty() {
                    vec![first.month()]
                } else {
                    bymonth.clone()
                };
                let v = months
                    .iter()
                    .flat_map(|&m| month_candidates(year, m, first.day()))
                    .collect();
                setpos(v)
            }
            _ => {
                // Tanınmayan sıklık: yalnızca ilk toplantı.
                out.push(start);
                break;
            }
        };
        let mut past_end = false;
        for d in dates {
            let at = d.and_time(time);
            if at < start {
                continue;
            }
            if until.is_some_and(|u| at > u) || count.is_some_and(|c| emitted >= c) {
                return out;
            }
            emitted += 1;
            if at > window_end {
                past_end = true;
                break;
            }
            out.push(at);
        }
        if past_end {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        n(s).and_utc()
    }

    fn occ(rule: &str, start: &str, end: &str) -> Vec<String> {
        expand(rule, n(start), n(end), |u| u.naive_utc())
            .into_iter()
            .map(|d| d.format("%Y-%m-%d %a").to_string())
            .collect()
    }

    #[test]
    fn expands_common_outlook_rules() {
        // Her hafta Pazartesi ve Çarşamba, 4 kez.
        assert_eq!(
            occ(
                "FREQ=WEEKLY;COUNT=4;BYDAY=MO,WE;WKST=SU",
                "2026-10-05 10:00",
                "2027-01-01 00:00"
            ),
            [
                "2026-10-05 Mon",
                "2026-10-07 Wed",
                "2026-10-12 Mon",
                "2026-10-14 Wed"
            ]
        );
        // İki haftada bir Salı, UNTIL'e kadar (dahil).
        assert_eq!(
            occ(
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261103T070000Z",
                "2026-10-06 10:00",
                "2027-01-01 00:00"
            ),
            ["2026-10-06 Tue", "2026-10-20 Tue"]
        );
        // Her ayın son Cuması.
        assert_eq!(
            occ(
                "FREQ=MONTHLY;BYDAY=FR;BYSETPOS=-1;COUNT=3",
                "2026-10-30 15:00",
                "2027-06-01 00:00"
            ),
            ["2026-10-30 Fri", "2026-11-27 Fri", "2026-12-25 Fri"]
        );
        // Her ayın 2. Salısı (Outlook'un diğer yazımı).
        assert_eq!(
            occ(
                "FREQ=MONTHLY;BYDAY=2TU;COUNT=2",
                "2026-10-13 09:00",
                "2027-06-01 00:00"
            ),
            ["2026-10-13 Tue", "2026-11-10 Tue"]
        );
        // Ayın 31'i olmayan aylar atlanır.
        assert_eq!(
            occ(
                "FREQ=MONTHLY;BYMONTHDAY=31",
                "2026-10-31 09:00",
                "2027-01-02 00:00"
            ),
            ["2026-10-31 Sat", "2026-12-31 Thu"]
        );
        // Hafta içi her gün; pencere sonunda durur (sonsuz kural).
        assert_eq!(
            occ(
                "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR",
                "2026-10-08 09:00",
                "2026-10-13 23:00"
            ),
            [
                "2026-10-08 Thu",
                "2026-10-09 Fri",
                "2026-10-12 Mon",
                "2026-10-13 Tue"
            ]
        );
        // Yıllık: Ekim'in ilk Pazartesisi.
        assert_eq!(
            occ(
                "FREQ=YEARLY;BYMONTH=10;BYDAY=1MO;COUNT=2",
                "2026-10-05 09:00",
                "2030-01-01 00:00"
            ),
            ["2026-10-05 Mon", "2027-10-04 Mon"]
        );
    }

    const OUTLOOK: &str = "BEGIN:VCALENDAR\r
METHOD:PUBLISH\r
PRODID:Microsoft Exchange Server 2010\r
VERSION:2.0\r
BEGIN:VTIMEZONE\r
TZID:Turkey Standard Time\r
BEGIN:STANDARD\r
DTSTART:16010101T000000\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T000000\r
TZOFFSETFROM:+0300\r
TZOFFSETTO:+0300\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VTIMEZONE\r
TZID:W. Europe Standard Time\r
BEGIN:STANDARD\r
DTSTART:16010101T030000\r
TZOFFSETFROM:+0200\r
TZOFFSETTO:+0100\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10\r
END:STANDARD\r
BEGIN:DAYLIGHT\r
DTSTART:16010101T020000\r
TZOFFSETFROM:+0100\r
TZOFFSETTO:+0200\r
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3\r
END:DAYLIGHT\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
RRULE:FREQ=WEEKLY;UNTIL=20261102T070000Z;INTERVAL=1;BYDAY=MO;WKST=MO\r
EXDATE;TZID=Turkey Standard Time:20261019T100000\r
UID:040000008200E00074C5B7101A82E0080000000001\r
SUMMARY;LANGUAGE=tr-TR:Togg haftalık\\, durum\r
DTSTART;TZID=Turkey Standard Time:20261005T100000\r
DTEND;TZID=Turkey Standard Time:20261005T103000\r
LOCATION;LANGUAGE=tr-TR:Microsoft Teams Toplantısı\r
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r
END:VEVENT\r
BEGIN:VEVENT\r
RECURRENCE-ID;TZID=Turkey Standard Time:20261012T100000\r
UID:040000008200E00074C5B7101A82E0080000000001\r
SUMMARY:Togg haftalık\\, durum\r
DTSTART;TZID=Turkey Standard Time:20261012T140000\r
DTEND;TZID=Turkey Standard Time:20261012T150000\r
LOCATION:Microsoft Teams Toplantısı\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:berlin\r
SUMMARY:Berlin atölye\r
DTSTART;TZID=W. Europe Standard Time:20261026T090000\r
DTEND;TZID=W. Europe Standard Time:20261026T110000\r
LOCATION:Toplantı odası 3\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:summer\r
SUMMARY:Yaz saati\r
DTSTART;TZID=W. Europe Standard Time:20260715T090000\r
DURATION:PT45M\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:allday\r
SUMMARY:Bayram\r
DTSTART;VALUE=DATE:20261029\r
DTEND;VALUE=DATE:20261030\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:free\r
SUMMARY:Öğle\r
DTSTART:20261005T090000Z\r
DTEND:20261005T100000Z\r
X-MICROSOFT-CDO-BUSYSTATUS:FREE\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:cancel\r
SUMMARY:İptal edildi: Kickoff\r
DTSTART:20261006T090000Z\r
DTEND:20261006T100000Z\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:long-desc\r
SUMMARY:Uzun açıklama\r
DTSTART:20261007T090000Z\r
DTEND:20261007T093000Z\r
DESCRIPTION:Katıl: https://teams.micro\r
 soft.com/l/meetup-join/abc\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn parses_outlook_calendar_with_zones_and_exceptions() {
        let cal = Calendar::parse(OUTLOOK);
        let got: Vec<(String, String, String, bool)> = cal
            .meetings(utc("2026-01-01 00:00"), utc("2027-01-01 00:00"))
            .into_iter()
            .map(|m| {
                (
                    m.start.format("%m-%d %H:%M").to_string(),
                    m.end.format("%H:%M").to_string(),
                    m.subject,
                    m.online,
                )
            })
            .collect();
        let row = |s: &str, e: &str, subject: &str, online: bool| {
            (s.to_string(), e.to_string(), subject.to_string(), online)
        };
        assert_eq!(
            got,
            [
                // Yaz saatinde Berlin UTC+2.
                row("07-15 07:00", "07:45", "Yaz saati", false),
                // Türkiye UTC+3: 10:00 → 07:00Z.
                row("10-05 07:00", "07:30", "Togg haftalık, durum", true),
                // Satır katlanması birleştirilir: açıklamadaki Teams linki.
                row("10-07 09:00", "09:30", "Uzun açıklama", true),
                // 12 Ekim tek seferlik 14:00'e alındı.
                row("10-12 11:00", "12:00", "Togg haftalık, durum", true),
                // 19 Ekim atlandı (EXDATE). 26 Ekim'de Berlin kış saatine döndü (UTC+1).
                row("10-26 07:00", "07:30", "Togg haftalık, durum", true),
                row("10-26 08:00", "10:00", "Berlin atölye", false),
                // UNTIL 2 Kasım 07:00Z (dahil).
                row("11-02 07:00", "07:30", "Togg haftalık, durum", true),
            ]
        );
        // Aralık süzgeci: yalnızca kesişenler.
        let day = cal.meetings(utc("2026-10-26 00:00"), utc("2026-10-26 07:15"));
        assert_eq!(day.len(), 1);
        assert_eq!(day[0].uid, "040000008200E00074C5B7101A82E0080000000001");
    }

    #[test]
    fn parses_small_pieces() {
        assert_eq!(parse_offset("+0300"), Some(10_800));
        assert_eq!(parse_offset("-0430"), Some(-16_200));
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_byday("-1SU"), Some((-1, Weekday::Sun)));
        assert_eq!(parse_byday("MO"), Some((0, Weekday::Mon)));
        assert_eq!(unescape(r"a\, b\; c\nd\\e"), "a, b; c\nd\\e");
        let line = parse_line(r#"DTSTART;TZID="Ev: Saat":20261005T100000"#).unwrap();
        assert_eq!(line.param("TZID"), Some("Ev: Saat"));
        assert_eq!(line.value, "20261005T100000");
    }
}
