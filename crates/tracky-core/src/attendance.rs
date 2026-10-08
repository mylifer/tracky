//! Toplantıya katılım ve gerçek süre: takvimdeki davet değil, görüşmenin kendisi
//! ([`crate::calls`]) ve o sırada yapılan iş.
//!
//! - Katılmadığın toplantı: çevrim içi toplantı süresince hiç görüşme yoksa ve sürenin en az
//!   yarısında başka işte çalışıldıysa toplantı yapılmamış sayılır; süre o işe kalır. Yalnızca
//!   görüşmeleri kaydeden bilgisayarlarda geçen toplantılar yargılanır (`monitored`).
//! - Gerçek süre: görüşme toplantıdan [`SLACK`]'ten fazla geç başladıysa, erken bittiyse ya da
//!   uzadıysa toplantının süresi görüşmeninki olur. Uzama en çok [`MAX_OVERRUN`] ve bir sonraki
//!   toplantının başlangıcına kadardır.
//! - Çakışan toplantılar: ortak süre, görüşme penceresinin başlığında konusu görünen toplantıya
//!   yazılır (yoksa eskisi gibi önce başlayana).
//!
//! Kullanıcının cevabı (katıldım / katılmadım) her zaman önce gelir.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::timesheet::{Interval, Meeting};

/// Görüşmenin toplantıdan bu kadar sapması önemsenmez (biraz geç katılma, erken ayrılma).
pub const SLACK: Duration = Duration::minutes(5);
/// Uzayan toplantı en çok bu kadar uzatılır.
pub const MAX_OVERRUN: Duration = Duration::minutes(60);
/// Toplantının en az bu oranında başka işte çalışıldıysa (ve görüşme yoksa) katılınmamıştır.
const SKIP_WORK_SHARE: f64 = 0.5;
/// Farklı bilgisayarlardaki görüşmeler bu kadar boşlukla birleşir.
const MERGE_GAP: Duration = Duration::minutes(2);
/// Konusu bundan kısa toplantı pencere başlığında aranmaz (yanlış eşleşmesin).
const MIN_SUBJECT_CHARS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Katılındı: görüşme var ya da kullanıcı söyledi.
    Attended,
    /// Katılınmadı: zaman çizelgesine girmez.
    Skipped,
    /// Bilinmiyor (görüşme kaydı yok, toplantı sürüyor): davetteki gibi sayılır.
    Unknown,
}

/// Bir toplantının katılımı.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendance {
    /// Cevabın anahtarı ([`key`]).
    pub key: String,
    pub status: Status,
    /// Kum çıkardı (kullanıcı cevaplamadı).
    pub auto: bool,
    /// Zaman çizelgesine giren aralık (görüşmeye göre düzeltilmiş).
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Toplantı süresindeki görüşmenin aralığı (ilk başlangıç, son bitiş).
    pub call: Option<Interval>,
    /// Kum'un kararının kısa gerekçesi (arayüzde gösterilir).
    pub reason: Option<String>,
}

/// Toplantının bu tekrarının kimliği: seri + başlangıç (cevaplar buna göre saklanır).
pub fn key(m: &Meeting) -> String {
    format!("{}@{}", m.uid, m.start.timestamp())
}

/// Karar için gerekenler.
pub struct Evidence<'a> {
    /// Görüşmeler (bütün bilgisayarlar).
    pub calls: &'a [Interval],
    /// Görüşme dışı çalışma (boşta olmayan, toplantı uygulaması olmayan oturumlar).
    pub work: &'a [Interval],
    /// Görüşme uygulaması pencereleri: (aralık, başlık).
    pub windows: &'a [(Interval, String)],
    /// Kullanıcının cevapları: anahtar → katıldı mı.
    pub answers: &'a HashMap<String, bool>,
    /// Toplantı süresince kullanılan bilgisayarlar görüşmeleri kaydediyor muydu?
    pub monitored: &'a dyn Fn(&Meeting) -> bool,
    pub now: DateTime<Utc>,
}

/// Toplantıların katılımı (aynı sırayla).
pub fn assess(meetings: &[Meeting], ev: &Evidence) -> Vec<Attendance> {
    let calls = merge(ev.calls.to_vec(), MERGE_GAP);
    let work = merge(ev.work.to_vec(), Duration::zero());
    let mut out: Vec<Attendance> = meetings
        .iter()
        .map(|m| one(m, meetings, &calls, &work, ev))
        .collect();
    resolve_overlaps(meetings, &mut out, ev);
    out
}

fn one(
    m: &Meeting,
    all: &[Meeting],
    calls: &[Interval],
    work: &[Interval],
    ev: &Evidence,
) -> Attendance {
    let answer = ev.answers.get(&key(m)).copied();
    let mut a = Attendance {
        key: key(m),
        status: Status::Unknown,
        auto: answer.is_none(),
        start: m.start,
        end: m.end,
        call: None,
        reason: None,
    };
    if answer == Some(false) {
        a.status = Status::Skipped;
        return a;
    }
    let within: Vec<Interval> = calls
        .iter()
        .copied()
        .filter(|&(c, d)| c < m.end && d > m.start)
        .collect();
    let ended = m.end <= ev.now;
    if let (Some(first), Some(last)) = (
        within.iter().map(|s| s.0).min(),
        within.iter().map(|s| s.1).max(),
    ) {
        a.status = Status::Attended;
        a.call = Some((first, last));
        if first > m.start + SLACK {
            a.start = first;
        }
        // Süren toplantının bitişi henüz bilinmez.
        if ended && last < m.end - SLACK {
            a.end = last.max(a.start);
        } else if last > m.end + SLACK {
            // Başka bir toplantıya geçen görüşme onun süresini almaz (sonraki ya da bitişten
            // sonra da süren çakışan toplantı).
            let next = all
                .iter()
                .filter(|o| !std::ptr::eq(*o, m) && o.end > m.end && o.start < last)
                .map(|o| o.start.max(m.end))
                .min();
            a.end = [Some(last), Some(m.end + MAX_OVERRUN), next]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(m.end)
                .max(m.end);
        }
        if (a.start, a.end) != (m.start, m.end) {
            a.reason = Some("Süre görüşmeye göre".into());
        }
        return a;
    }
    if answer == Some(true) {
        a.status = Status::Attended;
        return a;
    }
    let length = (m.end - m.start).num_seconds();
    let worked: i64 = work
        .iter()
        .map(|&(c, d)| (d.min(m.end) - c.max(m.start)).num_seconds().max(0))
        .sum();
    if m.online
        && ended
        && length > 0
        && worked as f64 >= SKIP_WORK_SHARE * length as f64
        && (ev.monitored)(m)
    {
        a.status = Status::Skipped;
        a.reason = Some("Görüşme yoktu, başka işte çalıştın".into());
    }
    a
}

/// Çakışan toplantılardan yalnızca biri görüşme penceresinde görünüyorsa ortak süre onundur:
/// öteki ortak süreden çıkarılır (tamamen içindeyse katılınmamış sayılır).
fn resolve_overlaps(meetings: &[Meeting], out: &mut [Attendance], ev: &Evidence) {
    for i in 0..meetings.len() {
        for j in 0..meetings.len() {
            if i == j || out[i].status == Status::Skipped || out[j].status == Status::Skipped {
                continue;
            }
            let (a, b) = (&out[i], &out[j]);
            let shared = (a.start.max(b.start), a.end.min(b.end));
            if shared.1 <= shared.0 {
                continue;
            }
            // `i` kazanır: penceresi görünüyor, `j`'ninki görünmüyor; kullanıcının cevabı
            // olan toplantıya dokunulmaz.
            let shown = |k: usize| in_window(&meetings[k], shared, ev.windows);
            if !shown(i) || shown(j) || !out[j].auto {
                continue;
            }
            let (win, lose) = ((out[i].start, out[i].end), (out[j].start, out[j].end));
            let lose_ref = &mut out[j];
            if win.0 <= lose.0 && lose.1 <= win.1 {
                lose_ref.status = Status::Skipped;
                lose_ref.reason = Some(format!(
                    "O sırada görüşmede “{}” vardı",
                    meetings[i].subject.trim()
                ));
            } else if lose.0 < win.0 {
                lose_ref.end = win.0;
            } else {
                lose_ref.start = win.1;
            }
        }
    }
}

/// Görüşme uygulamasının penceresinde (aralıkta) toplantının konusu görünüyor mu?
fn in_window(m: &Meeting, span: Interval, windows: &[(Interval, String)]) -> bool {
    let subject = normalize(&m.subject);
    if subject.chars().count() < MIN_SUBJECT_CHARS {
        return false;
    }
    windows
        .iter()
        .filter(|((c, d), _)| *c < span.1 && *d > span.0)
        .any(|(_, title)| {
            // Teams: "Konu | Microsoft Teams"; uzun konu başlıkta kısalabilir.
            let main = normalize(title.split(" | ").next().unwrap_or(title));
            main.contains(&subject)
                || (main.chars().count() >= MIN_SUBJECT_CHARS * 3 && subject.contains(&main))
        })
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .replace('\u{307}', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Aralıkları sıralayıp `gap`'ten kısa boşlukla ayrılanları birleştirir.
fn merge(mut spans: Vec<Interval>, gap: Duration) -> Vec<Interval> {
    spans.sort();
    let mut out: Vec<Interval> = Vec::new();
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 + gap => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Zaman çizelgesine giren toplantılar: katılınmayanlar çıkar, süreler düzeltilir. `att`
/// toplantıların katılımıdır ([`assess`]; anahtarla eşlenir).
pub fn apply(known: Vec<(Meeting, String)>, att: &[Attendance]) -> Vec<(Meeting, String)> {
    let by_key: HashMap<&str, &Attendance> = att.iter().map(|a| (a.key.as_str(), a)).collect();
    known
        .into_iter()
        .filter_map(|(mut m, p)| match by_key.get(key(&m).as_str()) {
            Some(a) if a.status == Status::Skipped => None,
            Some(a) => {
                m.start = a.start;
                m.end = a.end;
                (m.end > m.start).then_some((m, p))
            }
            None => Some((m, p)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    /// 2026-10-05 09:00 UTC'den itibaren dakika.
    fn t(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap() + Duration::minutes(min)
    }

    fn meeting(uid: &str, subject: &str, from: i64, to: i64) -> Meeting {
        Meeting {
            uid: uid.into(),
            start: t(from),
            end: t(to),
            subject: subject.into(),
            online: true,
            ..Meeting::default()
        }
    }

    struct Case {
        calls: Vec<Interval>,
        work: Vec<Interval>,
        windows: Vec<(Interval, String)>,
        answers: HashMap<String, bool>,
        monitored: bool,
        now: DateTime<Utc>,
    }

    impl Case {
        fn new() -> Self {
            Case {
                calls: Vec::new(),
                work: Vec::new(),
                windows: Vec::new(),
                answers: HashMap::new(),
                monitored: true,
                now: t(24 * 60),
            }
        }

        fn run(&self, meetings: &[Meeting]) -> Vec<Attendance> {
            let monitored = self.monitored;
            assess(
                meetings,
                &Evidence {
                    calls: &self.calls,
                    work: &self.work,
                    windows: &self.windows,
                    answers: &self.answers,
                    monitored: &move |_| monitored,
                    now: self.now,
                },
            )
        }
    }

    #[test]
    fn no_call_while_working_elsewhere_is_skipped() {
        let m = [meeting("a", "Haftalık", 60, 120)];
        let mut c = Case::new();
        c.work = vec![(t(55), t(100))];
        let a = &c.run(&m)[0];
        assert_eq!((a.status, a.auto), (Status::Skipped, true));
        // Az çalışıldıysa (odada toplantı) bilinmiyor: davetteki gibi.
        c.work = vec![(t(60), t(80))];
        assert_eq!(c.run(&m)[0].status, Status::Unknown);
        // Görüşmeleri kaydetmeyen bilgisayar yargılanmaz.
        c.work = vec![(t(60), t(120))];
        c.monitored = false;
        assert_eq!(c.run(&m)[0].status, Status::Unknown);
        // Yüz yüze toplantı da yargılanmaz.
        c.monitored = true;
        let f2f = [Meeting {
            online: false,
            ..m[0].clone()
        }];
        assert_eq!(c.run(&f2f)[0].status, Status::Unknown);
        // Süren toplantı henüz yargılanmaz.
        c.now = t(100);
        assert_eq!(c.run(&m)[0].status, Status::Unknown);
    }

    #[test]
    fn answers_win() {
        let m = [meeting("a", "Haftalık", 60, 120)];
        let mut c = Case::new();
        c.work = vec![(t(60), t(120))];
        c.answers.insert(key(&m[0]), true);
        let a = &c.run(&m)[0];
        assert_eq!((a.status, a.auto), (Status::Attended, false));
        c.answers.insert(key(&m[0]), false);
        c.calls = vec![(t(60), t(120))];
        let a = &c.run(&m)[0];
        assert_eq!((a.status, a.auto), (Status::Skipped, false));
    }

    #[test]
    fn duration_follows_the_call() {
        let m = [
            meeting("a", "Haftalık", 60, 120),
            meeting("b", "Sonraki", 150, 180),
        ];
        let mut c = Case::new();
        // Erken biten: 35 dakikada bitti.
        c.calls = vec![(t(58), t(95))];
        let a = &c.run(&m)[0];
        assert_eq!((a.status, a.start, a.end), (Status::Attended, t(60), t(95)));
        assert_eq!(a.call, Some((t(58), t(95))));
        // Geç katılınan ve birkaç dakikalık sapma.
        c.calls = vec![(t(75), t(118))];
        let a = &c.run(&m)[0];
        assert_eq!((a.start, a.end), (t(75), t(120)));
        // Uzayan: sonraki toplantıya kadar.
        c.calls = vec![(t(60), t(170))];
        let a = &c.run(&m)[0];
        assert_eq!(a.end, t(150));
        // Sonraki toplantı yoksa görüşmenin sonuna, en çok bir saat.
        let alone = [m[0].clone()];
        assert_eq!(c.run(&alone)[0].end, t(170));
        c.calls = vec![(t(60), t(240))];
        assert_eq!(c.run(&alone)[0].end, t(180));
        // Kısa kesinti görüşmeyi bölmez; farklı bilgisayarlar birleşir.
        c.calls = vec![(t(60), t(80)), (t(81), t(119))];
        assert_eq!(c.run(&alone)[0].end, t(120));
        // Süren toplantı erken bitmiş sayılmaz.
        c.calls = vec![(t(60), t(90))];
        c.now = t(90);
        assert_eq!(c.run(&alone)[0].end, t(120));
    }

    #[test]
    fn overlapping_meeting_shown_in_the_call_window_wins() {
        let m = [
            meeting("a", "Charging Sync", 60, 120),
            meeting("b", "Trumore Pitch Deck", 90, 150),
        ];
        let mut c = Case::new();
        c.calls = vec![(t(60), t(150))];
        // Önce başlayan değil, penceresi görünen kazanır: a ortak süreden çıkar.
        c.windows = vec![(
            (t(92), t(140)),
            "Trumore Pitch Deck | Microsoft Teams".into(),
        )];
        let a = c.run(&m);
        assert_eq!((a[0].start, a[0].end), (t(60), t(90)));
        assert_eq!((a[1].start, a[1].end), (t(90), t(150)));
        // Tamamen içinde kalan katılınmamış sayılır.
        let inner = [
            meeting("a", "Charging Sync", 95, 110),
            meeting("b", "Trumore Pitch Deck", 90, 150),
        ];
        let a = c.run(&inner);
        assert_eq!((a[0].status, a[0].auto), (Status::Skipped, true));
        assert!(
            a[0].reason
                .as_deref()
                .unwrap()
                .contains("Trumore Pitch Deck")
        );
        // İkisi de görünmüyorsa eskisi gibi (önce başlayan).
        c.windows.clear();
        let a = c.run(&m);
        assert_eq!((a[0].end, a[1].start), (t(120), t(90)));
        // Cevaplanan toplantıya dokunulmaz.
        c.windows = vec![(
            (t(92), t(140)),
            "Trumore Pitch Deck | Microsoft Teams".into(),
        )];
        c.answers.insert(key(&m[0]), true);
        assert_eq!(c.run(&m)[0].end, t(120));
    }

    #[test]
    fn apply_drops_skipped_and_moves_times() {
        let m = meeting("a", "Haftalık", 60, 120);
        let n = meeting("b", "Diğer", 200, 230);
        let att = vec![
            Attendance {
                key: key(&m),
                status: Status::Attended,
                auto: true,
                start: t(60),
                end: t(95),
                call: None,
                reason: None,
            },
            Attendance {
                key: key(&n),
                status: Status::Skipped,
                auto: true,
                start: t(200),
                end: t(230),
                call: None,
                reason: None,
            },
        ];
        let out = apply(vec![(m, "p".into()), (n, "p".into())], &att);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].0.start, out[0].0.end), (t(60), t(95)));
    }
}
