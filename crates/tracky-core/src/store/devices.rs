//! Bilgisayarlar: adları (eşitlenen `device:<kimlik>` ayarları) ve raporda hangi sürenin
//! hangi bilgisayardan geldiği.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Result, Store, StoreError};
pub use crate::blocks::BlockDevice;
use crate::model::Session;
pub use crate::report::DeviceTotal;
use crate::report::Report;

/// Bilgisayar adlarının ayar anahtarı öneki; değer [`DeviceInfo`]. Eşitlenir (her bilgisayar
/// kendi satırını yazar, ad başka bilgisayardan da değiştirilebilir).
pub const DEVICE_KEY_PREFIX: &str = "device:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    /// `macos`, `windows` ...
    #[serde(default)]
    pub os: String,
}

/// Kayıtlı bir bilgisayar (ayarlarda listelemek ve yeniden adlandırmak için).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownDevice {
    pub id: String,
    pub name: String,
    pub os: String,
    pub current: bool,
}

impl Store {
    /// Bu bilgisayarı adıyla kaydeder; zaten kayıtlıysa (adı başka yerden verilmiş olabilir)
    /// dokunmaz.
    pub fn register_device(&self, name: &str, os: &str) -> Result<()> {
        let key = format!("{DEVICE_KEY_PREFIX}{}", self.device_id);
        if self.setting::<DeviceInfo>(&key)?.is_some() {
            return Ok(());
        }
        let name = name.trim();
        let name = if name.is_empty() {
            default_name(os)
        } else {
            name
        };
        self.save_setting(
            &key,
            &DeviceInfo {
                name: name.into(),
                os: os.into(),
            },
        )
    }

    /// Bir bilgisayarın adını değiştirir (bu ya da başka bir bilgisayar).
    pub fn rename_device(&self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid("bilgisayar adı boş".into()));
        }
        Uuid::parse_str(id).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let key = format!("{DEVICE_KEY_PREFIX}{id}");
        // Adı hiç kaydedilmemiş bilgisayarın türü tahminden gelir.
        let os = self
            .resolve_names(&[id.to_string()])?
            .remove(id)
            .map(|d| d.os)
            .unwrap_or_default();
        self.save_setting(
            &key,
            &DeviceInfo {
                name: name.into(),
                os,
            },
        )
    }

    /// Kayıtlı bilgisayarlar ve kaydı olmayan ama oturumu bulunanlar (tahmini adla).
    pub fn known_devices(&self) -> Result<Vec<KnownDevice>> {
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT DISTINCT device_id FROM sessions WHERE deleted_at IS NULL")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut ids: HashSet<String> = ids.into_iter().collect();
        ids.extend(self.device_names()?.into_keys());
        ids.insert(self.device_id.to_string());
        let names = self.resolve_names(&ids.into_iter().collect::<Vec<_>>())?;
        let mut out: Vec<KnownDevice> = names
            .into_iter()
            .map(|(id, info)| KnownDevice {
                current: id == self.device_id.to_string(),
                id,
                name: info.name,
                os: info.os,
            })
            .collect();
        out.sort_by(|a, b| b.current.cmp(&a.current).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    /// `device:` ayarlarındaki adlar.
    fn device_names(&self) -> Result<HashMap<String, DeviceInfo>> {
        let rows: Vec<(String, String)> = self
            .conn
            .prepare("SELECT key, value FROM settings WHERE key LIKE 'device:%'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(key, value)| {
                let id = key.strip_prefix(DEVICE_KEY_PREFIX)?.to_string();
                Some((id, serde_json::from_str(&value).ok()?))
            })
            .collect())
    }

    /// Bilgisayarların görünen adları. Adı kaydedilmemiş (eski sürüm) bilgisayarın türü
    /// uygulama yollarından tahmin edilir; aynı adlar kimliğin başıyla ayrılır.
    fn resolve_names(&self, ids: &[String]) -> Result<HashMap<String, DeviceInfo>> {
        let known = self.device_names()?;
        let mut out: HashMap<String, DeviceInfo> = HashMap::new();
        for id in ids {
            let info = match known.get(id) {
                Some(info) => info.clone(),
                None => {
                    let windows: bool = self.conn.query_row(
                        "SELECT EXISTS (SELECT 1 FROM sessions WHERE device_id = ?1
                           AND substr(app_id, 2, 2) = ':\\')",
                        params![id],
                        |r| r.get(0),
                    )?;
                    let os = if windows { "windows" } else { "macos" };
                    DeviceInfo {
                        name: default_name(os).into(),
                        os: os.into(),
                    }
                }
            };
            out.insert(id.clone(), info);
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for info in out.values() {
            *seen.entry(info.name.clone()).or_default() += 1;
        }
        for (id, info) in &mut out {
            if seen[&info.name] > 1 {
                info.name = format!("{} ({})", info.name, &id[..id.len().min(4)]);
            }
        }
        Ok(out)
    }

    /// [`Store::report`]; `device` verilirse yalnızca o bilgisayarın kaydettiği süre. Aralıkta
    /// birden çok bilgisayar varsa rapor bilgisayar başına toplamları (filtresiz) ve
    /// blokların bilgisayar dağılımını da taşır.
    pub fn report_for_device(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        with_timeline: bool,
        device: Option<&str>,
    ) -> Result<Report> {
        let raw = self.sessions_with_devices_between(from, to)?;
        let device_of: HashMap<Uuid, String> = raw.iter().map(|(s, d)| (s.id, d.clone())).collect();
        let all: Vec<Session> = raw.into_iter().map(|(s, _)| s).collect();
        let merged_all = crate::model::merge_devices(all.clone());
        let sessions = match device {
            Some(d) => crate::model::merge_devices(
                all.into_iter()
                    .filter(|s| device_of.get(&s.id).is_some_and(|x| x == d))
                    .collect(),
            ),
            None => merged_all.clone(),
        };
        let mut report = self.build_report(&sessions, from, to, day_starts, with_timeline)?;

        let totals = seconds_by_device(&merged_all, &device_of, from, to);
        if totals.len() < 2 {
            return Ok(report);
        }
        let ids: Vec<String> = totals.keys().cloned().collect();
        let names = self.resolve_names(&ids)?;
        let own = self.device_id.to_string();
        let mut devices: Vec<DeviceTotal> = totals
            .into_iter()
            .map(|(id, seconds)| {
                let info = &names[&id];
                DeviceTotal {
                    current: id == own,
                    name: info.name.clone(),
                    os: info.os.clone(),
                    id,
                    seconds,
                }
            })
            .collect();
        devices.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.name.cmp(&b.name)));
        report.devices = devices;
        for block in &mut report.work.blocks {
            let mut per: Vec<BlockDevice> =
                seconds_by_device(&sessions, &device_of, block.start, block.end)
                    .into_iter()
                    .filter(|(_, secs)| *secs > 0)
                    .map(|(id, seconds)| BlockDevice {
                        name: names.get(&id).map_or_else(String::new, |i| i.name.clone()),
                        id,
                        seconds,
                    })
                    .collect();
            per.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.name.cmp(&b.name)));
            // Bir dakikadan kısa kırıntılar (başka bilgisayarda bir bildirime bakmak) dağılımı
            // kalabalıklaştırmasın; en büyük pay her zaman kalır.
            let mut first = true;
            per.retain(|d| std::mem::take(&mut first) || d.seconds >= MIN_BLOCK_SHARE_SECS);
            block.devices = per;
        }
        Ok(report)
    }
}

/// Birleştirilmiş (çakışmasız) oturumlarda `[from, to)` içindeki çalışma süresi, bilgisayar başına.
fn seconds_by_device(
    sessions: &[Session],
    device_of: &HashMap<Uuid, String>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> HashMap<String, i64> {
    let mut ms: HashMap<String, i64> = HashMap::new();
    for s in sessions.iter().filter(|s| s.counts_as_work()) {
        let span = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
        if span <= 0 {
            continue;
        }
        if let Some(d) = device_of.get(&s.id) {
            *ms.entry(d.clone()).or_default() += span;
        }
    }
    ms.into_iter().map(|(d, v)| (d, v / 1000)).collect()
}

/// Blok kartında gösterilen en kısa bilgisayar payı.
const MIN_BLOCK_SHARE_SECS: i64 = 60;

fn default_name(os: &str) -> &'static str {
    match os {
        "windows" => "Windows PC",
        "macos" => "Mac",
        "linux" => "Linux",
        _ => "Bilgisayar",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::seconds(secs)
    }

    fn session(app_id: &str, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app_id.into(),
            app_name: app_id.into(),
            title: String::new(),
            url: None,
            domain: None,
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
        }
    }

    /// Oturumu başka bir bilgisayarınmış gibi kaydeder.
    fn foreign(store: &Store, s: &Session, device: &str) {
        store.upsert_session(s).unwrap();
        store
            .conn
            .execute(
                "UPDATE sessions SET device_id = ?1 WHERE id = ?2",
                params![device, s.id.to_string()],
            )
            .unwrap();
    }

    const WIN: &str = "00000000-0000-4000-8000-0000000000aa";

    #[test]
    fn single_device_reports_carry_no_device_info() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_session(&session("code", 0, 600)).unwrap();
        let r = store
            .report_for_device(t(0), t(3600), &[t(0)], true, None)
            .unwrap();
        assert!(r.devices.is_empty());
        assert!(r.work.blocks.iter().all(|b| b.devices.is_empty()));
    }

    #[test]
    fn devices_are_named_totalled_and_filterable() {
        let store = Store::open_in_memory().unwrap();
        store.register_device("Mac Studio", "macos").unwrap();
        store.upsert_session(&session("code", 0, 600)).unwrap();
        // Adı kaydedilmemiş Windows bilgisayarı: türü uygulama yolundan tahmin edilir.
        foreign(&store, &session(r"C:\Figma.exe", 600, 1800), WIN);

        let r = store
            .report_for_device(t(0), t(3600), &[t(0)], true, None)
            .unwrap();
        let names: Vec<(&str, i64, bool)> = r
            .devices
            .iter()
            .map(|d| (d.name.as_str(), d.seconds, d.current))
            .collect();
        assert_eq!(
            names,
            vec![("Windows PC", 1200, false), ("Mac Studio", 600, true)]
        );
        let block_devices: i64 = r
            .work
            .blocks
            .iter()
            .flat_map(|b| &b.devices)
            .map(|d| d.seconds)
            .sum();
        assert_eq!(block_devices, 1800);

        // Filtre: yalnızca Windows'un süresi; bilgisayar listesi filtresiz kalır.
        let only = store
            .report_for_device(t(0), t(3600), &[t(0)], true, Some(WIN))
            .unwrap();
        assert_eq!(only.total_seconds, 1200);
        assert_eq!(only.devices.len(), 2);

        store.rename_device(WIN, "Ofis PC").unwrap();
        let r = store
            .report_for_device(t(0), t(3600), &[t(0)], true, None)
            .unwrap();
        assert_eq!(r.devices[0].name, "Ofis PC");
        assert_eq!(r.devices[0].os, "windows");
    }

    #[test]
    fn registering_keeps_a_name_given_elsewhere() {
        let store = Store::open_in_memory().unwrap();
        let own = store.device_id().to_string();
        store.rename_device(&own, "Çalışma Mac'i").unwrap();
        store.register_device("Kaan's Mac Studio", "macos").unwrap();
        let devices = store.known_devices().unwrap();
        assert_eq!(devices[0].name, "Çalışma Mac'i");
        assert!(devices[0].current);
    }
}
