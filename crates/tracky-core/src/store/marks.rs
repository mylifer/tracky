//! Zaman çizelgesine gönderilen takvim blokları ([`crate::timesheet::Sent`]): yalnızca bunların
//! işi canlı öneri olur. Gönderilen blok, gönderildiği andaki aralığıyla saklanır (`sent`); blok
//! sonradan değişse de bu aralıkla kesiştikçe gönderilmiş sayılır (satırı bloğu izler).
//!
//! `since` boş aralığı (kimliği `since:<bilgisayar>`) o bilgisayarın blokları seçerek göndermeye
//! geçtiği andır: bundan önce başlayan bloklar gönderilmiş sayılır (önceki sürümde canlı satır
//! olan iş yerinde kalır). Bilgisayarlar farklı zamanlarda güncellenir; en erken an geçerlidir.
//! Böyle bir blok çizelgeden çıkarılınca aralığı `unsent` olarak saklanır.
//!
//! Aralıklar eşitlenir (silme yumuşaktır).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use super::{Result, Store, StoreError, from_ms, ms};
use crate::timesheet::{self, Interval, Meeting, Sent};

/// Gönderilen blok.
const SENT: &str = "sent";
/// Seçerek göndermeden önceki, çizelgeden çıkarılan blok.
const UNSENT: &str = "unsent";
/// Bilgisayarın blokları seçerek göndermeye geçtiği an.
const SINCE: &str = "since";

/// Bloğu çizelgeden çıkarırken gizlenen satırlar (geri almada geri gelir) ve dosyaya
/// gönderilmiş olduğu için çizelgede kalan satır sayısı.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsent {
    pub hidden: Vec<String>,
    pub exported: usize,
}

/// `day_starts` (yerel gün sınırları) → (gün, başı ve sonu).
fn days(day_starts: &[DateTime<Utc>]) -> impl Iterator<Item = (chrono::NaiveDate, Interval)> + '_ {
    day_starts.windows(2).map(|w| {
        let date = w[0].with_timezone(&chrono::Local).date_naive();
        (date, (w[0], w[1]))
    })
}

/// Günün toplantıları.
fn todays(meetings: &[Meeting], (from, to): Interval) -> Vec<Meeting> {
    meetings
        .iter()
        .filter(|m| m.start < to && m.end > from)
        .cloned()
        .collect()
}

impl Store {
    /// Bu bilgisayarın blokları seçerek göndermeye geçtiği anı (ilk açılışta) kaydeder.
    pub(super) fn ensure_marks_since(&self) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "INSERT INTO timesheet_marks (id, kind, started_at, ended_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, ?3) ON CONFLICT (id) DO NOTHING",
            params![format!("{SINCE}:{}", self.device_id), SINCE, now],
        )?;
        Ok(())
    }

    /// Bu bilgisayarın blokları seçerek göndermeye geçtiği anı `at` yapar (testler ve veri
    /// düzeltmeleri için; normalde ilk açılışta kaydedilir).
    pub fn set_selective_since(&self, at: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE timesheet_marks SET started_at = ?2, ended_at = ?2,
                 updated_at = MAX(?3, updated_at + 1)
             WHERE id = ?1",
            params![
                format!("{SINCE}:{}", self.device_id),
                ms(at),
                ms(Utc::now())
            ],
        )?;
        Ok(())
    }

    /// `[from, to)` ile kesişen gönderilen ve çıkarılan bloklar, seçerek göndermeye geçilen an.
    pub fn timesheet_sent(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Sent> {
        let before: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(started_at) FROM timesheet_marks
                 WHERE deleted_at IS NULL AND kind = ?1",
                [SINCE],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let spans = |kind| -> Result<Vec<Interval>> {
            Ok(self
                .marks_between(kind, from, to)?
                .into_iter()
                .map(|(_, s)| s)
                .collect())
        };
        Ok(Sent {
            spans: spans(SENT)?,
            off: spans(UNSENT)?,
            // Geçiş anı bilinmiyorsa önceki davranış: her blok gönderilmiş.
            before: before.map_or(DateTime::<Utc>::MAX_UTC, from_ms),
        })
    }

    fn marks_between(
        &self,
        kind: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(String, Interval)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at FROM timesheet_marks
             WHERE deleted_at IS NULL AND kind = ?1 AND started_at < ?3 AND ended_at > ?2
             ORDER BY started_at",
        )?;
        let rows = stmt.query_map(params![kind, ms(from), ms(to)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, a, b) = row?;
            out.push((id, (from_ms(a), from_ms(b))));
        }
        Ok(out)
    }

    fn insert_mark(&self, kind: &str, (a, b): Interval) -> Result<()> {
        self.conn.execute(
            "INSERT INTO timesheet_marks (id, kind, started_at, ended_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                kind,
                ms(a),
                ms(b),
                ms(Utc::now())
            ],
        )?;
        Ok(())
    }

    /// `kind` aralıklarından `[from, to)` düşülür (dışında kalan kısımları, ör. komşu
    /// bloğunki, kalır). Değişen aralık sayısı.
    fn clip_marks(&self, kind: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        let now = ms(Utc::now());
        let marks = self.marks_between(kind, from, to)?;
        for (id, span) in &marks {
            self.conn.execute(
                "UPDATE timesheet_marks SET deleted_at = ?2, updated_at = MAX(?2, updated_at + 1)
                 WHERE id = ?1",
                params![id, now],
            )?;
            for left in timesheet::subtract(vec![*span], (from, to)) {
                self.insert_mark(kind, left)?;
            }
        }
        Ok(marks.len())
    }

    /// `[from, to)` gönderilen aralıklardan düşülür: blok kısaltılınca kesilen kısım.
    pub(crate) fn unsend_from_timesheet(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<usize> {
        self.clip_marks(SENT, from, to)
    }

    /// `[from, to)` gönderilmiş olur (çıkarılmışsa artık değil).
    fn mark_sent(&self, (from, to): Interval) -> Result<()> {
        if to <= from {
            return Err(StoreError::Invalid(
                "bitiş başlangıçtan sonra olmalı".into(),
            ));
        }
        self.clip_marks(UNSENT, from, to)?;
        self.insert_mark(SENT, (from, to))
    }

    /// Takvim bloğunu (`[from, to)`, projesi `project`) zaman çizelgesine gönderir: işi canlı
    /// öneri olur. Bilerek gönderilen iş, projenin silinmiş satırında kalsa da yeniden önerilir;
    /// yapılmadı sayılan toplantıların silinmiş satırları hariç. `day_starts` bloğun günlerinin
    /// yerel sınırları, `meetings` o günlerin takvim toplantıları.
    pub fn send_block(
        &self,
        (from, to): Interval,
        project: Option<&str>,
        day_starts: &[DateTime<Utc>],
        meetings: &[Meeting],
    ) -> Result<()> {
        let ctx = self.timesheet_context()?;
        let mut keep = HashSet::new();
        for (_, day) in days(day_starts) {
            let pieces = self.timesheet_pieces(&ctx, day.0, day.1, &todays(meetings, day))?;
            keep.extend(pieces.skipped().iter().cloned());
        }
        let tx = self.savepoint()?;
        self.mark_sent((from, to))?;
        if let Some(project) = project {
            self.forget_dismissed_except(project, from, to, &keep)?;
        }
        tx.commit()
    }

    /// Takvim bloğunu (`[from, to)`, projesi `project`) zaman çizelgesinden çıkarır: gönderilmemiş
    /// sayılır, projenin aralıktaki aktarılmamış kaydedilmiş satırları gizlenir. Toplantıların,
    /// elle eklenen ve dosyaya gönderilmiş satırlara dokunulmaz. `day_starts` ve `meetings`
    /// [`Self::send_block`]'taki gibi.
    pub fn remove_block(
        &self,
        (from, to): Interval,
        project: Option<&str>,
        day_starts: &[DateTime<Utc>],
        meetings: &[Meeting],
    ) -> Result<Unsent> {
        let tx = self.savepoint()?;
        self.clip_marks(SENT, from, to)?;
        if from < self.timesheet_sent(from, to)?.before {
            self.insert_mark(UNSENT, (from, to))?;
        }
        let mut out = Unsent::default();
        let ctx = self.timesheet_context()?;
        if let Some((project, sheet)) = project.and_then(|p| Some((p, ctx.config.timesheet_of(p)?)))
        {
            for (date, day) in days(day_starts) {
                let pieces = self.timesheet_pieces(&ctx, day.0, day.1, &todays(meetings, day))?;
                let meeting_spans: Vec<Interval> = pieces
                    .iter()
                    .filter(|p| p.meeting.is_some() && p.project == project)
                    .map(|p| (p.start, p.end))
                    .collect();
                for row in self.timesheet_day(&ctx, sheet, date, &pieces)?.rows {
                    let spans = row.entry.spans();
                    let inside = spans.iter().any(|&(a, b)| a < to && from < b);
                    let meeting = meeting_spans
                        .iter()
                        .fold(spans.clone(), |left, &m| timesheet::subtract(left, m))
                        .is_empty();
                    if row.entry.project_id != project || !inside || meeting {
                        continue;
                    }
                    if row.exported {
                        out.exported += 1;
                    } else {
                        out.hidden
                            .push(self.dismiss_timesheet_entry(row.id.as_deref(), &row.entry)?);
                    }
                }
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// Çizelgeden çıkarmayı ([`Self::remove_block`]) geri alır: `[from, to)` yeniden
    /// gönderilir, çıkarırken gizlenen satırlar geri gelir.
    pub fn restore_block(&self, span: Interval, hidden: &[String]) -> Result<()> {
        let tx = self.savepoint()?;
        self.mark_sent(span)?;
        self.undismiss_timesheet_entries(hidden)?;
        tx.commit()
    }
}
