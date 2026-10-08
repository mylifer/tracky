//! Görüşmeler: bir görüşme uygulamasının (Teams, Zoom, Webex, FaceTime ya da tarayıcıdaki Meet,
//! Teams) mikrofonu kullandığı aralıklar. Ön plandaki pencere toplantıya katılıp katılmadığını
//! söylemez (görüşme sürerken başka pencerede çalışılır); bu aralıklar söyler. Toplantının
//! katılımı ve gerçek süresi bunlardan çıkar ([`crate::attendance`]). Ses kaydedilmez; yalnızca
//! hangi uygulamanın ne zaman mikrofonu açık tuttuğu.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::platform::CallApps;
use crate::timesheet::TimesheetConfig;

/// Mikrofon bundan kısa süre kapanırsa (cihaz değişimi, yeniden bağlanma) aynı görüşme sürer.
pub const CALL_GAP: Duration = Duration::seconds(90);
/// Süren görüşme bu aralıkla diske yazılır (çökmede en fazla bu kadarı kaybolur).
const SAVE_EVERY: Duration = Duration::seconds(30);

/// Bir görüşme aralığı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub id: Uuid,
    /// Mikrofonu kullanan uygulama (macOS'ta paket kimliği).
    pub app_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

/// Toplantı uygulaması mı (zaman çizelgesi ayarlarındaki liste)?
fn is_meeting_app(app_id: &str, config: &TimesheetConfig) -> bool {
    let id = app_id.to_lowercase();
    config
        .meeting_apps
        .iter()
        .any(|p| crate::classify::app_matches(&p.to_lowercase(), &id))
}

/// Uygulama görüşme uygulaması mı: zaman çizelgesindeki toplantı uygulamaları ya da tarayıcı
/// (Meet, Teams'in web hâli). Mikrofonu sürekli açık tutan ses araçları (Boom 3D gibi) sayılmaz.
pub fn is_call_app(app_id: &str, config: &TimesheetConfig) -> bool {
    crate::browser::is_browser(app_id) || is_meeting_app(app_id, config)
}

/// Görüşmedeki uygulamalar: mikrofonu kullanan görüşme uygulamaları ve ekranı uyanık tutan
/// toplantı uygulamaları (sessize alınmış Teams; tarayıcının uyanık ekranı videodur, sayılmaz).
pub fn in_call(signals: &CallApps, config: &TimesheetConfig) -> Vec<String> {
    let mut out: Vec<String> = signals
        .microphone
        .iter()
        .filter(|a| is_call_app(a, config))
        .cloned()
        .collect();
    for a in signals.display.iter().filter(|a| is_meeting_app(a, config)) {
        if !out.contains(a) {
            out.push(a.clone());
        }
    }
    out
}

/// Yoklamalardan görüşme aralıkları çıkarır.
#[derive(Debug, Default)]
pub struct CallRecorder {
    current: Option<Call>,
    last_save: Option<DateTime<Utc>>,
}

impl CallRecorder {
    /// Yoklamanın sonucu: mikrofonu kullanan görüşme uygulamaları (boşsa görüşme yok). Diske
    /// yazılması gereken görüşmeler (yeni, uzayan ya da biten).
    pub fn observe(&mut self, now: DateTime<Utc>, apps: &[String]) -> Vec<Call> {
        let mut out = Vec::new();
        // Uzun boşluktan sonra süren görüşme son görüldüğü anda bitmiştir; son hâli yazılır.
        if self
            .current
            .as_ref()
            .is_some_and(|c| now - c.ended_at > CALL_GAP || now < c.started_at)
        {
            out.extend(self.current.take());
        }
        let Some(app) = apps.first() else {
            return out;
        };
        match &mut self.current {
            Some(call) => {
                call.ended_at = now.max(call.ended_at);
                if self
                    .last_save
                    .is_none_or(|t| now - t >= SAVE_EVERY || now < t)
                {
                    self.last_save = Some(now);
                    out.push(call.clone());
                }
            }
            None => {
                let call = Call {
                    id: Uuid::new_v4(),
                    app_id: app.clone(),
                    started_at: now,
                    ended_at: now,
                };
                self.current = Some(call.clone());
                self.last_save = Some(now);
                out.push(call);
            }
        }
        out
    }

    /// Takip duraklatılınca ya da kapanırken: süren görüşme son görüldüğü anda biter.
    pub fn finish(&mut self) -> Option<Call> {
        self.current.take()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + s, 0).unwrap()
    }

    fn apps(on: bool) -> Vec<String> {
        if on {
            vec!["com.microsoft.teams2".into()]
        } else {
            Vec::new()
        }
    }

    #[test]
    fn call_apps_are_meeting_apps_and_browsers() {
        let config = TimesheetConfig::default();
        assert!(is_call_app("com.microsoft.teams2", &config));
        assert!(is_call_app("us.zoom.xos", &config));
        assert!(is_call_app("com.google.Chrome", &config));
        assert!(!is_call_app("com.globaldelight.Boom3D", &config));
    }

    #[test]
    fn short_gaps_keep_the_call_and_long_ones_end_it() {
        let mut r = CallRecorder::default();
        let first = r.observe(t(0), &apps(true)).pop().unwrap();
        // Süren görüşme ancak yazma aralığında bir yazılır.
        assert_eq!(r.observe(t(5), &apps(true)), []);
        let saved = r.observe(t(30), &apps(true)).pop().unwrap();
        assert_eq!((saved.id, saved.ended_at), (first.id, t(30)));
        // 60 sn mikrofon kapalı: aynı görüşme.
        assert_eq!(r.observe(t(60), &apps(false)), []);
        let saved = r.observe(t(90), &apps(true));
        assert_eq!((saved[0].id, saved[0].ended_at), (first.id, t(90)));
        // Uzun boşluk: görüşme son görüldüğü anda biter.
        assert_eq!(r.observe(t(150), &apps(false)), []);
        let ended = r.observe(t(200), &apps(false));
        assert_eq!(ended.len(), 1);
        assert_eq!((ended[0].id, ended[0].ended_at), (first.id, t(90)));
        assert_eq!(r.observe(t(205), &apps(false)), []);
        // Yeni görüşme yeni kimlikle.
        let next = r.observe(t(300), &apps(true)).pop().unwrap();
        assert_ne!(next.id, first.id);
        // Boşluktan sonra hemen yeni görüşme: eskisinin son hâli de yazılır.
        let both = r.observe(t(500), &apps(true));
        assert_eq!(both.len(), 2);
        assert_eq!((both[0].id, both[0].ended_at), (next.id, t(300)));
        assert_eq!(r.finish().map(|c| c.id), Some(both[1].id));
    }

    #[test]
    fn muted_meeting_app_counts_but_a_browser_video_does_not() {
        let config = TimesheetConfig::default();
        let signals = CallApps {
            microphone: vec![
                "com.globaldelight.Boom3D".into(),
                "com.google.Chrome".into(),
            ],
            display: vec!["com.microsoft.teams2".into(), "com.apple.Safari".into()],
        };
        assert_eq!(
            in_call(&signals, &config),
            ["com.google.Chrome", "com.microsoft.teams2"]
        );
    }
}
