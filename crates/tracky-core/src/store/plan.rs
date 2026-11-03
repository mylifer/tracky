//! Gün planı: kullanıcının onayladığı zaman blokları.
//!
//! Takvimin eski blokları ([`crate::blocks`]) her açılışta ham oturumlardan yeniden hesaplanır;
//! bir bloğu küçültmek oturumlara "burada yeni blok başlasın" işareti yazar ve sonuç kaç parçaya
//! bölüneceği belli olmayan yeni bir hesaptır. Plan blokları ise saklanan kayıtlardır (Timely'nin
//! girişleri, ManicTime'ın etiketleri gibi): yalnızca kullanıcı değiştirince değişir, kenarı
//! sürüklenince yalnızca kenarı oynar. Ham oturumlar kanıt olarak kalır.
//!
//! Arayüz bir günün bloklarını bütün olarak kaydeder ([`Store::replace_plan`]); önceki hali
//! döner ki değişiklik geri alınabilsin. Silme yumuşaktır (eşitlemeye hazır).

use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Result, Store, StoreError, from_ms, ms};

/// Plan bloğu: `[start, end)` aralığı, projesi (yoksa projesiz) ve notu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanBlock {
    /// Boş verilirse kaydederken yeni kimlik alır.
    #[serde(default)]
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub note: String,
}

impl Store {
    /// `[from, to)` ile kesişen plan blokları, başlangıca göre sıralı.
    pub fn plan_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<PlanBlock>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, project_id, note FROM plan_blocks
             WHERE deleted_at IS NULL AND started_at < ?2 AND ended_at > ?1
             ORDER BY started_at, id",
        )?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok(PlanBlock {
                id: r.get(0)?,
                start: from_ms(r.get(1)?),
                end: from_ms(r.get(2)?),
                project_id: r.get(3)?,
                note: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `[from, to)`'daki plan bloklarını `blocks` yapar: listede olmayanlar silinir, kimliği
    /// olanlar güncellenir (silinmişse geri gelir), kimliksizler eklenir. Bloklar aralığın
    /// içinde olmalı ve üst üste binmemeli. Önceki bloklar döner (geri almak için aynı işlevle
    /// yeniden kaydedilir).
    pub fn replace_plan(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        blocks: &[PlanBlock],
    ) -> Result<Vec<PlanBlock>> {
        let mut sorted: Vec<&PlanBlock> = blocks.iter().collect();
        sorted.sort_by_key(|b| b.start);
        for b in &sorted {
            if b.end <= b.start {
                return Err(StoreError::Invalid(
                    "bloğun bitişi başlangıcından sonra olmalı".into(),
                ));
            }
            if b.start < from || b.end > to {
                return Err(StoreError::Invalid("blok günün dışına taşıyor".into()));
            }
        }
        if sorted.windows(2).any(|w| w[1].start < w[0].end) {
            return Err(StoreError::Invalid("bloklar üst üste biniyor".into()));
        }
        for id in sorted.iter().filter_map(|b| b.project_id.as_deref()) {
            if id != crate::classify::NO_PROJECT {
                self.require_tag(id, crate::classify::TagKind::Project)?;
            }
        }

        let previous = self.plan_between(from, to)?;
        let now = ms(Utc::now());
        let tx = self.savepoint()?;
        for old in &previous {
            if !sorted.iter().any(|b| b.id == old.id) {
                self.conn.execute(
                    "UPDATE plan_blocks SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
                     WHERE id = ?1",
                    params![old.id, now],
                )?;
            }
        }
        for b in sorted {
            let id = if b.id.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                b.id.clone()
            };
            self.conn.execute(
                "INSERT INTO plan_blocks (id, started_at, ended_at, project_id, note, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     started_at = excluded.started_at, ended_at = excluded.ended_at,
                     project_id = excluded.project_id, note = excluded.note, deleted_at = NULL,
                     updated_at = MAX(excluded.updated_at, plan_blocks.updated_at + 1)",
                params![id, ms(b.start), ms(b.end), b.project_id, b.note, now],
            )?;
        }
        tx.commit()?;
        Ok(previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(min: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::minutes(min)
    }

    fn block(id: &str, a: i64, b: i64, project: Option<&str>) -> PlanBlock {
        PlanBlock {
            id: id.into(),
            start: t(a),
            end: t(b),
            project_id: project.map(str::to_string),
            note: String::new(),
        }
    }

    #[test]
    fn saves_updates_and_removes_blocks() {
        let store = Store::open_in_memory().unwrap();
        let project = store.accept_project_suggestion("Togg").unwrap();
        let p = Some(project.id.as_str());

        let prev = store
            .replace_plan(
                t(0),
                t(600),
                &[block("", 60, 180, p), block("", 200, 260, None)],
            )
            .unwrap();
        assert!(prev.is_empty());
        let saved = store.plan_between(t(0), t(600)).unwrap();
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().all(|b| !b.id.is_empty()));
        assert_eq!(saved[0].project_id.as_deref(), p);

        // Kenar sürüklenir (kimlik aynı), ikinci blok silinir.
        let mut first = saved[0].clone();
        first.end = t(240);
        let prev = store.replace_plan(t(0), t(600), &[first.clone()]).unwrap();
        assert_eq!(prev, saved);
        assert_eq!(store.plan_between(t(0), t(600)).unwrap(), vec![first]);

        // Geri alma: önceki hal aynı işlevle kaydedilir; silinen blok kimliğiyle geri gelir.
        store.replace_plan(t(0), t(600), &prev).unwrap();
        assert_eq!(store.plan_between(t(0), t(600)).unwrap(), saved);
    }

    #[test]
    fn only_touches_the_given_range() {
        let store = Store::open_in_memory().unwrap();
        store
            .replace_plan(t(0), t(1440), &[block("", 60, 120, None)])
            .unwrap();
        store
            .replace_plan(t(1440), t(2880), &[block("", 1500, 1560, None)])
            .unwrap();
        store.replace_plan(t(0), t(1440), &[]).unwrap();
        let left = store.plan_between(t(0), t(2880)).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].start, t(1500));
    }

    #[test]
    fn rejects_invalid_blocks() {
        let store = Store::open_in_memory().unwrap();
        let overlap = [block("", 0, 60, None), block("", 30, 90, None)];
        assert!(store.replace_plan(t(0), t(600), &overlap).is_err());
        assert!(
            store
                .replace_plan(t(0), t(600), &[block("", 60, 60, None)])
                .is_err()
        );
        assert!(
            store
                .replace_plan(t(0), t(600), &[block("", 550, 650, None)])
                .is_err()
        );
        assert!(
            store
                .replace_plan(t(0), t(600), &[block("", 0, 60, Some("yok"))])
                .is_err()
        );
        assert!(store.plan_between(t(0), t(600)).unwrap().is_empty());
    }
}
