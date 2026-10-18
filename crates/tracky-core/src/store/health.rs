//! Veri denetimi: kayıtlarda hiç olmaması gerekenler. Testler hepsi yeşilken bile gerçek
//! hatalar canlı verinin sorgularından çıktı (aynı bilgisayarda üst üste binen oturumlar,
//! gece boyu süren "görüşme"); bu denetim onları uygulamanın içinde arar ve onarılabilenleri
//! onarır. Aktarılmış çizelge satırlarının işine ve hâlâ süren kayıtlara dokunulmaz.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use super::{Result, Store, from_ms, ms};
use crate::calls::CALL_AWAY;
use crate::model::{IDLE_APP_ID, MANUAL_APP_ID};

/// Bundan kısa üst üste binme ölçüm payıdır (iki pencerenin aynı saniyede kapanıp açılması).
const OVERLAP_SLACK_MS: i64 = 1_500;
/// Bundan yeni kayıtlar sürüyor olabilir (bu ya da öteki bilgisayar uzatıyor): dokunulmaz.
const LIVE_MS: i64 = 15 * 60 * 1000;
/// Onarım bölünen oturumun kalanı başka bir oturumla da çakışabilir: en çok bu kadar tur.
const REPAIR_ROUNDS: usize = 5;

/// Denetimin bulguları.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    /// Aynı bilgisayarda üst üste binen oturumlar (süre iki kez sayılır).
    pub overlaps: Vec<Overlap>,
    /// Kullanıcı bilgisayar başında değilken de süren görüşmeler (toplantı katılımını şaşırtır).
    pub long_calls: Vec<LongCall>,
    /// Aynı işi kapsayan çizelge satırı çiftleri (iş iki kez yazılır); kendiliğinden onarılmaz.
    pub double_rows: Vec<DoubleRow>,
}

impl Health {
    pub fn is_clean(&self) -> bool {
        self.overlaps.is_empty() && self.long_calls.is_empty() && self.double_rows.is_empty()
    }

    /// Onarılabilecek bulgu sayısı.
    pub fn repairable(&self) -> usize {
        self.overlaps.iter().filter(|o| o.repairable).count() + self.long_calls.len()
    }

    /// Günlük ve tanılama için tek satır.
    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "sorun yok".into();
        }
        let mut parts = Vec::new();
        if !self.overlaps.is_empty() {
            parts.push(format!("{} çakışan oturum", self.overlaps.len()));
        }
        if !self.long_calls.is_empty() {
            parts.push(format!("{} aşırı uzun görüşme", self.long_calls.len()));
        }
        if !self.double_rows.is_empty() {
            parts.push(format!(
                "aynı işi kapsayan {} satır çifti",
                self.double_rows.len()
            ));
        }
        parts.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlap {
    pub device_id: String,
    /// Önce başlayan oturum ve uygulaması.
    pub first: String,
    pub first_app: String,
    /// Sonra başlayan, aralığını alan oturum ve uygulaması.
    pub second: String,
    pub second_app: String,
    pub start: DateTime<Utc>,
    pub seconds: i64,
    /// Onarılabilir mi (aktarılmış satırın işinde ya da süren kayıtta değil).
    pub repairable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LongCall {
    pub id: String,
    pub app_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Bilgisayar başında olunan son an; görüşme buraya kısalır (başlangıçtaysa silinir).
    pub until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubleRow {
    pub date: String,
    pub first: String,
    pub second: String,
    pub minutes: i64,
}

/// Onarımın sonucu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repaired {
    pub sessions: usize,
    pub calls: usize,
}

/// Oturumun denetimde gerekenleri.
#[derive(Debug, Clone)]
struct Span {
    id: String,
    device: String,
    app: String,
    start: i64,
    end: i64,
}

impl Store {
    /// Kayıtları denetler (değiştirmez).
    pub fn health(&self) -> Result<Health> {
        let now = ms(Utc::now());
        let exported = self.exported_spans()?;
        let touches_exported = |a: i64, b: i64| exported.iter().any(|&(x, y)| x < b && a < y);
        let overlaps = self
            .overlapping_pairs()?
            .into_iter()
            .map(|(p, n)| {
                let (a, b) = (n.start, p.end.min(n.end));
                Overlap {
                    repairable: !touches_exported(a, b) && p.end.max(n.end) < now - LIVE_MS,
                    device_id: p.device.clone(),
                    first: p.id,
                    first_app: p.app,
                    second: n.id,
                    second_app: n.app,
                    start: from_ms(a),
                    seconds: (b - a) / 1000,
                }
            })
            .collect();
        Ok(Health {
            overlaps,
            long_calls: self.long_calls(now)?,
            double_rows: self.double_rows()?,
        })
    }

    /// Onarılabilen bulguları onarır: üst üste binen oturumda sonra başlayan aralığını alır
    /// (öncekinden o kısım çıkar, gerekirse ikiye bölünür); uzun görüşme bilgisayar başında
    /// olunan son ana kısalır. Tek işlemde; değişen satırlar eşitlenir.
    pub fn repair_health(&self) -> Result<Repaired> {
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        let mut done = Repaired::default();
        let mut health = self.health()?;
        // Bölünen oturumun kalanı sonraki bir oturumla da çakışıyorsa bir tur daha.
        for _ in 0..REPAIR_ROUNDS {
            let mut changed = 0;
            for o in health.overlaps.iter().filter(|o| o.repairable) {
                changed += self.carve(&o.first, &o.second, now)?;
            }
            done.sessions += changed;
            health = self.health()?;
            if changed == 0 {
                break;
            }
        }
        for c in &health.long_calls {
            let until = ms(c.until);
            done.calls += if until <= ms(c.started_at) {
                self.conn.execute(
                    "UPDATE calls SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
                     WHERE id = ?1 AND deleted_at IS NULL",
                    params![c.id, now],
                )?
            } else {
                self.conn.execute(
                    "UPDATE calls SET ended_at = ?2, updated_at = MAX(?3, updated_at + 1)
                     WHERE id = ?1 AND ended_at > ?2",
                    params![c.id, until, now],
                )?
            };
        }
        tx.commit()?;
        Ok(done)
    }

    /// Aynı bilgisayarda önceki oturumun bitişi sonrakinin başlangıcını geçen çiftler (önceki,
    /// sonraki). Elle kayıtlar dışarıda: bilerek eklenen aralık takiple çakışabilir.
    fn overlapping_pairs(&self) -> Result<Vec<(Span, Span)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, app_id, started_at, ended_at FROM sessions
             WHERE deleted_at IS NULL AND substr(app_id, 1, length(?1)) != ?1
             ORDER BY device_id, started_at, id",
        )?;
        let rows: Vec<Span> = stmt
            .query_map(params![format!("{MANUAL_APP_ID}/")], |r| {
                Ok(Span {
                    id: r.get(0)?,
                    device: r.get(1)?,
                    app: r.get(2)?,
                    start: r.get(3)?,
                    end: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        // Bilgisayar başına, bitişi en ileri giden önceki oturum.
        let mut reach: Option<&Span> = None;
        for n in &rows {
            if let Some(p) = reach.filter(|p| p.device == n.device)
                && p.end - n.start > OVERLAP_SLACK_MS
            {
                out.push((p.clone(), n.clone()));
            }
            if reach.is_none_or(|p| p.device != n.device || n.end > p.end) {
                reach = Some(n);
            }
        }
        Ok(out)
    }

    /// `second`'in aralığını `first`'ten çıkarır; `first` ondan sonra da sürüyorsa kalan kısmı
    /// yeni bir oturum olur. Değişen satır sayısı.
    fn carve(&self, first: &str, second: &str, now: i64) -> Result<usize> {
        let span = |id: &str| -> Result<Option<(i64, i64)>> {
            Ok(self
                .conn
                .query_row(
                    "SELECT started_at, ended_at FROM sessions WHERE id = ?1 AND deleted_at IS NULL",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok())
        };
        // Önceki onarımlar aralıkları değiştirmiş olabilir: güncel hâllerine bakılır.
        let (Some((a, b)), Some((c, d))) = (span(first)?, span(second)?) else {
            return Ok(0);
        };
        if b - c <= OVERLAP_SLACK_MS {
            return Ok(0);
        }
        let mut changed = 0;
        if b > d {
            // Kalan kuyruk, sonraki oturumdan sonra (ilk oturumun her alanıyla).
            changed += self.conn.execute(
                "INSERT INTO sessions (id, device_id, app_id, app_name, title, url, domain,
                     category_id, project_id, state_at, block_from, passive, started_at, ended_at,
                     updated_at)
                 SELECT ?2, device_id, app_id, app_name, title, url, domain, category_id,
                     project_id, state_at, block_from, passive, ?3, ?4, ?5
                 FROM sessions WHERE id = ?1",
                params![first, Uuid::new_v4().to_string(), d, b, now],
            )?;
        }
        changed += if c <= a {
            self.conn.execute(
                "UPDATE sessions SET deleted_at = ?2, state_at = ?2,
                     updated_at = MAX(?2, updated_at + 1)
                 WHERE id = ?1",
                params![first, now],
            )?
        } else {
            self.conn.execute(
                "UPDATE sessions SET ended_at = ?2, updated_at = MAX(?3, updated_at + 1)
                 WHERE id = ?1",
                params![first, c, now],
            )?
        };
        Ok(changed)
    }

    /// Kullanıcı bilgisayar başında değilken de süren görüşmeler: görüşmenin bilgisayarında,
    /// görüşme içinde [`CALL_AWAY`]'den uzun süre hiç çalışma (boşta olmayan oturum) yoksa
    /// görüşme o aradan önceki son çalışmada biter.
    fn long_calls(&self, now: i64) -> Result<Vec<LongCall>> {
        let away = CALL_AWAY.num_milliseconds();
        let calls: Vec<(String, String, String, i64, i64)> = self
            .conn
            .prepare(
                "SELECT id, device_id, app_id, started_at, ended_at FROM calls
                 WHERE deleted_at IS NULL AND ended_at - started_at > ?1 AND ended_at < ?2",
            )?
            .query_map(params![away, now - LIVE_MS], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, device, app_id, a, b) in calls {
            let presence: Vec<(i64, i64)> = self
                .conn
                .prepare(
                    "SELECT started_at, ended_at FROM sessions
                     WHERE device_id = ?1 AND deleted_at IS NULL AND app_id != ?2
                       AND started_at < ?4 AND ended_at > ?3
                     ORDER BY started_at",
                )?
                .query_map(params![device, IDLE_APP_ID, a, b], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let until = present_until(a, b, &presence, away);
            if until < b {
                out.push(LongCall {
                    id,
                    app_id,
                    started_at: from_ms(a),
                    ended_at: from_ms(b),
                    until: from_ms(until),
                });
            }
        }
        Ok(out)
    }

    /// Aktarılmış (dosyaya yazılmış) çizelge satırlarının kapsadığı takip aralıkları.
    fn exported_spans(&self) -> Result<Vec<(i64, i64)>> {
        let covers: Vec<Option<String>> = self
            .conn
            .prepare(
                "SELECT coverage FROM timesheet_entries
                 WHERE exported_at IS NOT NULL AND deleted_at IS NULL",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(covers
            .into_iter()
            .flatten()
            .filter_map(|c| serde_json::from_str::<Vec<[i64; 2]>>(&c).ok())
            .flatten()
            .map(|[a, b]| (a, b))
            .collect())
    }

    /// Kapsadığı takip aralıkları bir dakikadan uzun kesişen, gizlenmemiş çizelge satırı çiftleri.
    fn double_rows(&self) -> Result<Vec<DoubleRow>> {
        let rows: Vec<(String, String, Option<String>)> = self
            .conn
            .prepare(
                "SELECT id, date, coverage FROM timesheet_entries
                 WHERE deleted_at IS NULL AND dismissed_at IS NULL AND coverage IS NOT NULL",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut spans: Vec<(i64, i64, usize)> = Vec::new();
        for (i, (_, _, cover)) in rows.iter().enumerate() {
            for [a, b] in cover
                .as_deref()
                .and_then(|c| serde_json::from_str::<Vec<[i64; 2]>>(c).ok())
                .unwrap_or_default()
            {
                spans.push((a, b, i));
            }
        }
        spans.sort_unstable();
        let mut shared: HashMap<(usize, usize), i64> = HashMap::new();
        for (k, &(a, b, i)) in spans.iter().enumerate() {
            for &(c, d, j) in &spans[k + 1..] {
                if c >= b {
                    break;
                }
                if i != j {
                    *shared.entry((i.min(j), i.max(j))).or_default() += b.min(d) - c.max(a);
                }
            }
        }
        let mut out: Vec<DoubleRow> = shared
            .into_iter()
            .filter(|(_, ms)| *ms > Duration::minutes(1).num_milliseconds())
            .map(|((i, j), ms)| DoubleRow {
                date: rows[i].1.clone(),
                first: rows[i].0.clone(),
                second: rows[j].0.clone(),
                minutes: ms / 60_000,
            })
            .collect();
        out.sort_by(|x, y| x.date.cmp(&y.date).then(x.first.cmp(&y.first)));
        Ok(out)
    }
}

/// Görüşme `[a, b)` içinde bilgisayar başında olunan son an: çalışma aralıkları (`presence`,
/// başlangıca göre) arasında `away`'den uzun boşluk ilk nerede başlıyorsa orası.
fn present_until(a: i64, b: i64, presence: &[(i64, i64)], away: i64) -> i64 {
    let mut reach = a;
    for &(s, e) in presence {
        if s.max(a) - reach > away {
            return reach;
        }
        reach = reach.max(e.min(b));
    }
    if b - reach > away { reach } else { b }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::calls::Call;
    use crate::model::Session;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::minutes(min)
    }

    fn session(app: &str, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.into(),
            title: String::new(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
            block_from: None,
            passive: false,
        }
    }

    /// Bu bilgisayardaki oturumlar (dakika), başlangıca göre.
    fn spans(store: &Store) -> Vec<(String, i64, i64)> {
        let mut out: Vec<(String, i64, i64)> = store
            .sessions_between(t(-1000), t(10_000))
            .unwrap()
            .into_iter()
            .map(|s| {
                let min = |x: DateTime<Utc>| (x - t(0)).num_minutes();
                (s.app_id, min(s.started_at), min(s.ended_at))
            })
            .collect();
        out.sort_by_key(|s| (s.1, s.2));
        out
    }

    #[test]
    fn overlapping_sessions_are_found_and_the_later_one_keeps_its_span() {
        let store = Store::open_in_memory().unwrap();
        // Girdi yokken değişen başlık boşta bloğunun içine taştı; boşta blok da sonraki işin içine.
        store.upsert_session(&session("code", 0, 10)).unwrap();
        store.upsert_session(&session(IDLE_APP_ID, 8, 40)).unwrap();
        store.upsert_session(&session("figma", 20, 25)).unwrap();
        store.upsert_session(&session("mail", 50, 60)).unwrap();
        let health = store.health().unwrap();
        assert_eq!(health.overlaps.len(), 2, "{health:?}");
        assert!(health.overlaps.iter().all(|o| o.repairable));
        assert_eq!(health.overlaps[0].seconds, 120);
        assert_eq!(health.repairable(), 2);

        let done = store.repair_health().unwrap();
        assert!(done.sessions >= 3, "{done:?}");
        assert_eq!(
            spans(&store),
            [
                ("code".to_string(), 0, 8),
                (IDLE_APP_ID.to_string(), 8, 20),
                ("figma".to_string(), 20, 25),
                (IDLE_APP_ID.to_string(), 25, 40),
                ("mail".to_string(), 50, 60),
            ]
        );
        assert!(store.health().unwrap().is_clean());
        assert_eq!(store.repair_health().unwrap(), Repaired::default());
    }

    #[test]
    fn other_devices_and_manual_entries_are_not_overlaps() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("code", 0, 30)).unwrap();
        let other = session("mail", 10, 20);
        store.upsert_session(&other).unwrap();
        store
            .conn()
            .execute(
                "UPDATE sessions SET device_id = 'öteki' WHERE id = ?1",
                params![other.id.to_string()],
            )
            .unwrap();
        store
            .upsert_session(&session(&format!("{MANUAL_APP_ID}/Toplantı"), 5, 15))
            .unwrap();
        assert!(store.health().unwrap().overlaps.is_empty());
    }

    #[test]
    fn exported_work_and_live_sessions_are_left_alone() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("code", 0, 10)).unwrap();
        store.upsert_session(&session(IDLE_APP_ID, 8, 40)).unwrap();
        // Aktarılmış satır bu işi kapsıyor: dosyadaki satır değişmesin diye onarılmaz.
        store
            .conn()
            .execute(
                "INSERT INTO timesheet_entries (id, date, start, hours, kind, details, party,
                     project_id, division, exported_at, created_at, coverage, updated_at)
                 VALUES ('r1', '2023-11-14', '22:13', 0.25, 'Working', 'x', 'p', 'proj', 'd',
                     1, 1, ?1, 1)",
                params![format!(
                    "[[{}, {}]]",
                    t(0).timestamp_millis(),
                    t(10).timestamp_millis()
                )],
            )
            .unwrap();
        let health = store.health().unwrap();
        assert_eq!(health.overlaps.len(), 1);
        assert!(!health.overlaps[0].repairable);
        assert_eq!(store.repair_health().unwrap(), Repaired::default());

        // Süren kayıt (bu ya da öteki bilgisayar uzatıyor olabilir) de onarılmaz.
        let fresh = Store::open_in_memory().unwrap();
        let now = Utc::now();
        let mut work = session("code", 0, 0);
        (work.started_at, work.ended_at) = (now - Duration::minutes(10), now);
        let mut idle = session(IDLE_APP_ID, 0, 0);
        (idle.started_at, idle.ended_at) = (now - Duration::minutes(5), now);
        fresh.upsert_session(&work).unwrap();
        fresh.upsert_session(&idle).unwrap();
        assert!(!fresh.health().unwrap().overlaps[0].repairable);
    }

    #[test]
    fn a_call_that_goes_on_after_the_user_left_is_cut_where_they_left() {
        let store = Store::open_in_memory().unwrap();
        let call = |from, to| Call {
            id: Uuid::new_v4(),
            app_id: "org.mozilla.firefox".into(),
            started_at: t(from),
            ended_at: t(to),
        };
        // Gece: 20 dk çalışma, sonra 14 saat hiçbir şey; mikrofon açık kaldı.
        store.upsert_session(&session("firefox", 0, 20)).unwrap();
        let night = call(0, 14 * 60);
        store.upsert_call(&night).unwrap();
        // Başında hiç çalışma olmayan görüşme bütünüyle sahte.
        let ghost = call(2000, 2100);
        store.upsert_call(&ghost).unwrap();
        // Gerçek toplantı: kesintisiz çalışma (görüşme penceresi), uzun ama sorun değil.
        store.upsert_session(&session("teams", 3000, 3090)).unwrap();
        store.upsert_call(&call(3000, 3090)).unwrap();

        let long = store.health().unwrap().long_calls;
        assert_eq!(long.len(), 2, "{long:?}");
        assert_eq!(long[0].until, t(20));
        assert_eq!(long[1].until, t(2000));
        assert_eq!(store.repair_health().unwrap().calls, 2);
        let calls = store.calls_between(t(-10), t(5000)).unwrap();
        let ranges: Vec<(i64, i64)> = calls
            .iter()
            .map(|c| {
                (
                    (c.started_at - t(0)).num_minutes(),
                    (c.ended_at - t(0)).num_minutes(),
                )
            })
            .collect();
        assert_eq!(ranges, [(0, 20), (3000, 3090)]);
        assert!(store.health().unwrap().is_clean());
    }

    #[test]
    fn rows_covering_the_same_work_are_reported() {
        let store = Store::open_in_memory().unwrap();
        let ms = |m| t(m).timestamp_millis();
        for (id, cover) in [
            ("a", format!("[[{}, {}]]", ms(0), ms(30))),
            (
                "b",
                format!("[[{}, {}], [{}, {}]]", ms(20), ms(40), ms(50), ms(60)),
            ),
            ("c", format!("[[{}, {}]]", ms(60), ms(70))),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO timesheet_entries (id, date, start, hours, kind, details, party,
                         project_id, division, created_at, coverage, updated_at)
                     VALUES (?1, '2023-11-14', '22:13', 0.5, 'Working', 'x', 'p', 'proj', 'd',
                         1, ?2, 1)",
                    params![id, cover],
                )
                .unwrap();
        }
        let health = store.health().unwrap();
        assert_eq!(
            health.double_rows,
            [DoubleRow {
                date: "2023-11-14".into(),
                first: "a".into(),
                second: "b".into(),
                minutes: 10,
            }]
        );
        assert_eq!(health.summary(), "aynı işi kapsayan 1 satır çifti");
        // Kendiliğinden onarılmaz: hangisinin doğru olduğuna kullanıcı karar verir.
        assert_eq!(health.repairable(), 0);
    }

    #[test]
    fn presence_ends_at_the_first_long_absence() {
        let away = 30;
        assert_eq!(present_until(0, 100, &[(0, 10), (20, 80)], away), 100);
        // Sondaki uzun yokluk da sayılır.
        assert_eq!(present_until(0, 100, &[(0, 10), (20, 30)], away), 30);
        assert_eq!(present_until(0, 100, &[(0, 10), (50, 60)], away), 10);
        assert_eq!(present_until(0, 100, &[], away), 0);
        // Görüşmeden önce başlayan çalışma da sayılır.
        assert_eq!(present_until(10, 100, &[(0, 80)], away), 100);
        assert_eq!(present_until(10, 200, &[(0, 80)], away), 80);
    }
}
