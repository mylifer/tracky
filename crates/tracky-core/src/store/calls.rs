//! Görüşmeler ([`crate::calls`]), toplantı katılımı cevapları ve bilgisayarların görüşmeleri
//! ne zamandan beri kaydettiği.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::params;
use uuid::Uuid;

use super::devices::DeviceInfo;
use super::{DEVICE_KEY_PREFIX, Result, Store, from_ms, ms};
use crate::calls::Call;
use crate::model::IDLE_APP_ID;

/// Kullanıcının toplantı katılımı cevapları (eşitlenir): [`crate::attendance::key`] → katıldı mı.
pub(super) const MEETING_ATTENDANCE_KEY: &str = "meeting_attendance";

impl Store {
    /// Görüşmeyi ekler ya da (süren görüşmede) bitişini günceller.
    pub fn upsert_call(&self, c: &Call) -> Result<()> {
        self.conn.execute(
            "INSERT INTO calls (id, device_id, app_id, started_at, ended_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                ended_at = MAX(calls.ended_at, excluded.ended_at),
                updated_at = MAX(excluded.updated_at, calls.updated_at + 1)",
            params![
                c.id.to_string(),
                self.device_id.to_string(),
                c.app_id,
                ms(c.started_at),
                ms(c.ended_at),
                ms(Utc::now()),
            ],
        )?;
        Ok(())
    }

    /// `[from, to)` ile kesişen görüşmeler (bütün bilgisayarlar), başlangıca göre.
    pub fn calls_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Call>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, app_id, started_at, ended_at FROM calls
             WHERE deleted_at IS NULL AND started_at < ?2 AND ended_at > ?1
             ORDER BY started_at",
        )?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, app_id, a, b) = row?;
            out.push(Call {
                id: Uuid::parse_str(&id).unwrap_or_default(),
                app_id,
                started_at: from_ms(a),
                ended_at: from_ms(b),
            });
        }
        Ok(out)
    }

    /// Bu bilgisayarın görüşmeleri kaydedip kaydetmediğini işler: açıksa başlangıç anı (ilk
    /// kez), kapalıysa silinir. Kaydetmeyen bilgisayarda geçen toplantı yargılanmaz.
    pub fn set_call_detection(&self, on: bool) -> Result<()> {
        let key = format!("{DEVICE_KEY_PREFIX}{}", self.device_id);
        let Some(mut info) = self.setting::<DeviceInfo>(&key)? else {
            return Ok(());
        };
        let next = match (on, info.calls_from) {
            (true, Some(since)) => Some(since),
            (true, None) => Some(Utc::now()),
            (false, _) => None,
        };
        if next != info.calls_from {
            info.calls_from = next;
            self.save_setting(&key, &info)?;
        }
        Ok(())
    }

    /// `[from, to)` boyunca kullanılan (boşta olmayan oturumu olan) bilgisayarların hepsi o
    /// andan önce görüşmeleri kaydetmeye başlamış mı? Hiç oturum yoksa yargılanamaz (`false`).
    pub fn calls_monitored(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<bool> {
        let devices: Vec<String> = self
            .conn
            .prepare(&format!(
                "SELECT DISTINCT device_id FROM sessions
                 WHERE {} AND deleted_at IS NULL AND app_id != ?3",
                super::sessions::OVERLAPS
            ))?
            .query_map(params![ms(from), ms(to), IDLE_APP_ID], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if devices.is_empty() {
            return Ok(false);
        }
        for id in devices {
            let info = self.setting::<DeviceInfo>(&format!("{DEVICE_KEY_PREFIX}{id}"))?;
            if !info
                .and_then(|i| i.calls_from)
                .is_some_and(|since| since <= from)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Toplantı katılımı cevapları.
    pub fn meeting_answers(&self) -> Result<HashMap<String, bool>> {
        Ok(self.setting(MEETING_ATTENDANCE_KEY)?.unwrap_or_default())
    }

    /// Toplantının bu tekrarına katılıp katılmadığı (`None`: cevabı geri al, Kum karar versin).
    pub fn answer_meeting(&self, key: &str, attended: Option<bool>) -> Result<()> {
        let mut all = self.meeting_answers()?;
        match attended {
            Some(a) => all.insert(key.to_string(), a),
            None => all.remove(key),
        };
        self.save_setting(MEETING_ATTENDANCE_KEY, &all)
    }
}
