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
/// DURATION bundan uzunsa bozuk sayılır (toplantı zaten [`MAX_LENGTH`]'ten kısadır); akıl
/// dışı değerler (P200000000D) toplamada taşmasın.
const MAX_DURATION: Duration = Duration::days(31);

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
    organizer: Option<String>,
    attendees: Vec<String>,
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
    /// Yıllık tekrarın bittiği an (yerel, geçişten önceki saatle; RRULE UNTIL). Kalkmış yaz
    /// saati kuralları (örn. Türkiye 2016) böyle yazılır.
    until: Option<NaiveDateTime>,
}

impl Default for Observance {
    fn default() -> Self {
        Self {
            start: NaiveDateTime::MIN,
            offset_from: 0,
            offset_to: 0,
            yearly: None,
            until: None,
        }
    }
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

/// ORGANIZER / ATTENDEE satırındaki e-posta: `mailto:` değeri, yoksa `EMAIL` parametresi.
/// Küçük harfe çevrilir; '@' içermeyen (oda, kaynak adı) değer atlanır.
fn email_of(line: &Line) -> Option<String> {
    let value = line.value.trim();
    let address = match value.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("mailto:") => &value[7..],
        _ => line.param("EMAIL").unwrap_or(value),
    };
    let address = address.trim().to_lowercase();
    let (user, domain) = address.split_once('@')?;
    (!user.is_empty() && domain.contains('.') && !address.contains(char::is_whitespace))
        .then_some(address)
}

fn parse_naive(v: &str) -> Option<NaiveDateTime> {
    let v = v.trim();
    if v.len() >= 15 {
        NaiveDateTime::parse_from_str(v.get(..15)?, "%Y%m%dT%H%M%S").ok()
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
    let h: i32 = digits.get(0..2)?.parse().ok()?;
    let m: i32 = digits.get(2..4)?.parse().ok()?;
    let s: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + s))
}

/// "PT1H30M", "P1D", "-PT15M" → süre. [`MAX_DURATION`]'dan uzunsa (ya da sayı taşarsa) `None`.
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
                // Dosyadan gelen sayı taşabilir: `Duration::days` vb. taşmada panikler.
                let part = match unit {
                    'W' => Duration::try_weeks(n),
                    'D' => Duration::try_days(n),
                    'H' => Duration::try_hours(n),
                    'M' => Duration::try_minutes(n),
                    'S' => Duration::try_seconds(n),
                    _ => return None,
                }?;
                total = total.checked_add(&part).filter(|t| *t <= MAX_DURATION)?;
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
    let (n, day) = v.split_at_checked(split)?;
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
        // Süren geçiş tanımı ve RRULE UNTIL'i (UTC mi?); UNTIL, TZOFFSETFROM'u gerektirir.
        let mut obs: Option<(Observance, Option<(NaiveDateTime, bool)>)> = None;
        // Etkinliğin içindeki bileşen derinliği (VALARM...): içlerindeki satırlar (alarmın
        // SUMMARY, ATTENDEE, DURATION'ı) etkinliğe uygulanmaz.
        let mut nested = 0usize;
        for raw in unfold(text) {
            let Some(line) = parse_line(&raw) else {
                continue;
            };
            let value = line.value;
            let upper = value.trim().to_ascii_uppercase();
            if event.is_some() && upper != "VEVENT" {
                match line.name.as_str() {
                    "BEGIN" => {
                        nested += 1;
                        continue;
                    }
                    "END" => {
                        nested = nested.saturating_sub(1);
                        continue;
                    }
                    _ if nested > 0 => continue,
                    _ => {}
                }
            }
            match (line.name.as_str(), upper.as_str()) {
                ("BEGIN", "VEVENT") => {
                    event = Some(Event::default());
                    nested = 0;
                }
                ("END", "VEVENT") => {
                    nested = 0;
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
                    obs = Some((Observance::default(), None));
                }
                ("END", "STANDARD" | "DAYLIGHT") => {
                    if let (Some((mut o, until)), Some((_, z))) = (obs.take(), &mut zone) {
                        // UNTIL (kural gereği UTC) geçişten önceki yerel saate çevrilir.
                        o.until = until.and_then(|(u, utc)| {
                            if utc {
                                u.checked_add_signed(Duration::seconds(o.offset_from.into()))
                            } else {
                                Some(u)
                            }
                        });
                        z.observances.push(o);
                    }
                }
                _ => {
                    if let Some((o, until)) = &mut obs {
                        match line.name.as_str() {
                            "DTSTART" => o.start = parse_naive(value).unwrap_or(o.start),
                            "TZOFFSETFROM" => {
                                o.offset_from = parse_offset(value).unwrap_or(o.offset_from)
                            }
                            "TZOFFSETTO" => {
                                o.offset_to = parse_offset(value).unwrap_or(o.offset_to)
                            }
                            "RRULE" => {
                                o.yearly = parse_yearly(value);
                                *until = rule_parts(value)
                                    .get("UNTIL")
                                    .and_then(|u| Some((parse_naive(u)?, u.ends_with('Z'))));
                            }
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

    /// Her toplantı serisinden (UID) bir örnek, tekrarlar açılmadan: serinin ilk hâli (tek
    /// seferlik değişiklik değil). Saat ilk tekrarınkidir. Toplantı önerileri geçmiş serilerden
    /// öğrenirken kullanılır ([`crate::meeting_suggest`]); tüm geçmişi açmaktan çok ucuzdur.
    pub fn series(&self) -> Vec<Meeting> {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut out: Vec<Meeting> = Vec::new();
        for e in &self.events {
            if e.skip || e.all_day || e.uid.is_empty() {
                continue;
            }
            let Some((start, length)) = self.span(e) else {
                continue;
            };
            let Some((s, end)) = self
                .to_utc(&start)
                .and_then(|s| Some((s, s.checked_add_signed(length)?)))
            else {
                continue;
            };
            let meeting = Meeting {
                uid: e.uid.clone(),
                start: s,
                end,
                subject: e.summary.trim().to_string(),
                location: e.location.trim().to_string(),
                online: e.online,
                organizer: e.organizer.clone(),
                attendees: e.attendees.clone(),
            };
            match seen.get(e.uid.as_str()) {
                // Değişiklik önce gelmişse yerine serinin asıl hâli yazılır.
                Some(&i) if e.recurrence_id.is_none() => out[i] = meeting,
                Some(_) => {}
                None => {
                    seen.insert(e.uid.as_str(), out.len());
                    out.push(meeting);
                }
            }
        }
        out
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
                    organizer: e.organizer.clone(),
                    attendees: e.attendees.clone(),
                });
            }
        };
        for e in &self.events {
            let Some((start, length)) = self.span(e) else {
                continue;
            };
            if e.recurrence_id.is_some() || e.rrule.is_none() {
                if let Some(s) = self.to_utc(&start)
                    && let Some(end) = s.checked_add_signed(length)
                {
                    push(e, s, end);
                }
                continue;
            }
            let Some(rule) = e.rrule.as_deref() else {
                continue;
            };
            let excluded: Vec<DateTime<Utc>> =
                e.exdates.iter().filter_map(|x| self.to_utc(x)).collect();
            // Açma, etkinliğin kendi dilimindeki yerel saatlerle yapılır (yaz saatinde de
            // toplantı aynı saatte kalır). Pencere bir gün genişletilir (dilim farkı).
            let until_local = |u: DateTime<Utc>| self.to_local_of(&start, u);
            let window_end = self
                .to_local_of(&start, to)
                .checked_add_signed(Duration::days(1))
                .unwrap_or(NaiveDateTime::MAX);
            for occ in expand(rule, start.naive(), window_end, until_local) {
                let Some(s) = self.to_utc(&start.with_naive(occ)) else {
                    continue;
                };
                if excluded.contains(&s) {
                    continue;
                }
                match (
                    overrides.get(&(e.uid.as_str(), s)),
                    s.checked_add_signed(length),
                ) {
                    (Some(_), _) | (_, None) => {} // değişen hâli ayrıca eklenir
                    (None, Some(end)) => push(e, s, end),
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
                Some(zone) => n
                    .checked_sub_signed(Duration::seconds(zone.offset_at(*n).into()))
                    .map(|n| n.and_utc()),
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
            if let Some(onset) = o.last_onset(local)
                && best.is_none_or(|(b, _)| onset > b)
            {
                best = Some((onset, o.offset_to));
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

impl Observance {
    /// `local` anına kadarki (dahil) son geçiş. Tekrarsız geçiş yalnızca bir kez olur; yıllık
    /// kural UNTIL'den sonra işlemez (bittiyse son geçişi UNTIL yılındadır).
    fn last_onset(&self, local: NaiveDateTime) -> Option<NaiveDateTime> {
        let Some(yearly) = self.yearly else {
            return (self.start <= local).then_some(self.start);
        };
        let mut years = vec![local.year() - 1, local.year()];
        if let Some(u) = self.until
            && u.year() < local.year() - 1
        {
            years.push(u.year());
        }
        years
            .into_iter()
            .filter_map(|year| yearly.date_in(year))
            .map(|d| d.and_time(self.start.time()))
            .filter(|&onset| {
                onset >= self.start && onset <= local && self.until.is_none_or(|u| onset <= u)
            })
            .max()
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
        NaiveDate::from_weekday_of_month_opt(year, month, wd, u8::try_from(n).ok()?)
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
            "ORGANIZER" => self.organizer = email_of(line),
            "ATTENDEE" => {
                if let Some(a) = email_of(line)
                    && !self.attendees.contains(&a)
                {
                    self.attendees.push(a);
                }
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
        // INTERVAL dosyadan gelir: çarpım taşarsa kural bitmiş sayılır.
        let Some(step) = (k as u32).checked_mul(interval) else {
            break;
        };
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
                let Some(year) = i32::try_from(step)
                    .ok()
                    .and_then(|s| first.year().checked_add(s))
                else {
                    break;
                };
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
mod tests;
