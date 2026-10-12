//! Rapor ve sorgular: arama, dışa aktarma, proje istatistiği, eğilimler, toplamlar.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{Result, Store, UsageTotal};
use crate::classify::Classifier;
use crate::model::{IDLE_APP_ID, Session};
use crate::report::{self, Report};

/// [`Store::project_totals_all_time`] için eski geçmişin proje toplamları (milisaniye).
pub(super) struct TotalsCache {
    cutoff: DateTime<Utc>,
    fingerprint: Vec<i64>,
    totals_ms: HashMap<String, i64>,
}

impl Store {
    /// `[from, to)` aralığında başlığında ya da uygulama adında `query` geçen süre.
    pub fn search(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
    ) -> Result<crate::search::SearchResult> {
        let sessions = self.merged_sessions_between(from, to)?;
        Ok(crate::search::search(
            &sessions, query, from, to, day_starts,
        ))
    }

    /// Aramayla eşleşen oturumların CSV dökümü; oturumlar aralığa kırpılır.
    pub fn export_search_csv(
        &self,
        query: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<String> {
        let needle = crate::search::fold(query.trim());
        let sessions: Vec<Session> = self
            .merged_sessions_between(from, to)?
            .into_iter()
            .filter(|s| !needle.is_empty() && crate::search::matches(s, &needle))
            .map(|mut s| {
                s.started_at = s.started_at.max(from);
                s.ended_at = s.ended_at.min(to);
                s
            })
            .filter(|s| s.ended_at > s.started_at)
            .collect();
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(crate::export::sessions_csv(&sessions, &tags, &classifier))
    }

    /// `[from, to)` aralığında projenin profili; `day_starts` yerel gün sınırları,
    /// `utc_offset` bir anın yerel saat farkı (saniye).
    pub fn project_stats(
        &self,
        project: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        utc_offset: impl Fn(DateTime<Utc>) -> i32,
    ) -> Result<crate::project_stats::ProjectStats> {
        let sessions = self.merged_sessions_between(from, to)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        Ok(crate::project_stats::build(
            &sessions,
            &classifier,
            project,
            from,
            to,
            day_starts,
            utc_offset,
        ))
    }

    /// Ardışık dönemlerde proje ve kategori süreleri; `bounds` dönem sınırları (n + 1 öğe).
    pub fn trends(&self, bounds: &[DateTime<Utc>]) -> Result<crate::trends::Trends> {
        let (Some(first), Some(last)) = (bounds.first(), bounds.last()) else {
            return Ok(Default::default());
        };
        let sessions = self.merged_sessions_between(*first, *last)?;
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        Ok(crate::trends::trends(&sessions, &classifier, bounds))
    }

    /// Tüm çalışma oturumlarının CSV dökümü (atanmamış boşta kayıtları hariç).
    pub fn export_csv(&self) -> Result<String> {
        let mut sessions =
            self.sessions_between(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC)?;
        sessions.retain(Session::counts_as_work);
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(crate::export::sessions_csv(&sessions, &tags, &classifier))
    }

    /// `[from, to)` raporu; `day_starts` yerel gün sınırlarıdır.
    pub fn report(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        with_timeline: bool,
    ) -> Result<Report> {
        self.report_for_device(from, to, day_starts, with_timeline, None)
    }

    /// Birleştirilmiş oturumlardan rapor.
    pub(super) fn build_report(
        &self,
        sessions: &[Session],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        day_starts: &[DateTime<Utc>],
        with_timeline: bool,
    ) -> Result<Report> {
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        Ok(report::build(
            sessions,
            &tags,
            &classifier,
            from,
            to,
            day_starts,
            with_timeline,
        ))
    }

    /// Kategori başına toplam süre (saniye); kategorisiz süre dahil edilmez.
    pub fn category_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        self.tag_totals(from, to, |c| c.category)
    }

    /// Proje başına toplam süre (saniye); projesiz süre dahil edilmez.
    pub fn project_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        self.tag_totals(from, to, |c| c.project)
    }

    fn tag_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        pick: fn(crate::classify::Classification) -> Option<String>,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let mut out = self.tag_totals_ms(from, to, pick)?;
        out.values_mut().for_each(|v| *v /= 1000);
        Ok(out)
    }

    /// [`Self::tag_totals`], milisaniye: oturum başına kırpılmaz, en sonda saniyeye çevrilir.
    fn tag_totals_ms(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        pick: fn(crate::classify::Classification) -> Option<String>,
    ) -> Result<HashMap<String, i64>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut out: HashMap<String, i64> = HashMap::new();
        for s in self.merged_sessions_between(from, to)? {
            let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
            if let (Some(id), true) = (pick(classifier.classify(&s)), ms > 0) {
                *out.entry(id).or_default() += ms;
            }
        }
        Ok(out)
    }

    /// Projelerin bugüne (`now`) kadarki tüm süresi (sözleşme bütçeleri için), saniye.
    ///
    /// Tüm geçmişi toplamak geçmiş büyüdükçe uzar ve bu sırada depo kilitli kalır (takip,
    /// menü çubuğu ve arayüz bekler). Bu yüzden iki günden eski kısmın toplamı saklanır ve
    /// yalnızca o kısma dokunan bir değişiklikte (eski oturuma atama, silme, eşitlemeyle
    /// gelen satır, kural ya da proje değişikliği) yeniden hesaplanır; son iki gün her
    /// seferinde toplanır. Aralıklar kesişen oturumları tam aldığı ve cihaz birleştirmesi
    /// aralıktan bağımsız olduğu için iki parçanın toplamı tek seferde toplamaya eşittir.
    pub fn project_totals_all_time(&self, now: DateTime<Utc>) -> Result<HashMap<String, i64>> {
        let pick: fn(crate::classify::Classification) -> Option<String> = |c| c.project;
        let cutoff = (now - chrono::Duration::days(2))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map_or(now, |t| t.and_utc());
        let fingerprint = self.history_fingerprint(cutoff)?;
        let cached = self
            .totals_cache
            .borrow()
            .as_ref()
            .filter(|c| c.cutoff == cutoff && c.fingerprint == fingerprint)
            .map(|c| c.totals_ms.clone());
        let mut out = match cached {
            Some(totals) => totals,
            None => {
                let totals = self.tag_totals_ms(DateTime::<Utc>::UNIX_EPOCH, cutoff, pick)?;
                *self.totals_cache.borrow_mut() = Some(TotalsCache {
                    cutoff,
                    fingerprint,
                    totals_ms: totals.clone(),
                });
                totals
            }
        };
        for (id, ms) in self.tag_totals_ms(cutoff, now, pick)? {
            *out.entry(id).or_default() += ms;
        }
        out.values_mut().for_each(|v| *v /= 1000);
        Ok(out)
    }

    /// `cutoff`'tan önce başlayan oturumlar ve sınıflandırmayı belirleyen etiket ve kurallar
    /// değişince değişen özet. Her satır değişikliği `updated_at`'i değiştirir; toplamlar
    /// taşmasın diye zaman damgaları bir asal sayıya göre kalanla toplanır.
    fn history_fingerprint(&self, cutoff: DateTime<Utc>) -> Result<Vec<i64>> {
        const P: i64 = 2_147_483_647;
        let mut out = Vec::with_capacity(13);
        let row =
            |sql: &str, n: usize, p: &[&dyn rusqlite::ToSql], out: &mut Vec<i64>| -> Result<()> {
                self.conn.query_row(sql, p, |r| {
                    for i in 0..n {
                        out.push(r.get::<_, Option<i64>>(i)?.unwrap_or(0));
                    }
                    Ok(())
                })?;
                Ok(())
            };
        row(
            "SELECT COUNT(*), MAX(updated_at), SUM(updated_at % ?2), SUM(ended_at % ?2),
                    SUM(COALESCE(state_at, 0) % ?2), SUM(COALESCE(deleted_at, 0) % ?2)
             FROM sessions WHERE started_at < ?1",
            6,
            &[&cutoff.timestamp_millis(), &P],
            &mut out,
        )?;
        row(
            "SELECT COUNT(*), SUM(updated_at % ?1), SUM(COALESCE(deleted_at, 0) % ?1),
                    SUM(COALESCE(archived_at, 0) % ?1)
             FROM tags",
            4,
            &[&P],
            &mut out,
        )?;
        row(
            "SELECT COUNT(*), SUM(updated_at % ?1), SUM(COALESCE(deleted_at, 0) % ?1) FROM rules",
            3,
            &[&P],
            &mut out,
        )?;
        Ok(out)
    }

    /// Son kullanılan uygulamalar (kural ve gizlilik seçicileri için), en yeni önce; ad en
    /// son kullanıldığı haliyle. Süre hesaplanmaz (`seconds` 0): tüm geçmişi toplamak her
    /// sayfa açılışında tabloyu tarardı. Uygulamalar (app_id, ended_at) indeksinde atlayarak
    /// bulunur ("loose index scan"): uygulama başına bir arama.
    pub fn known_apps(&self, limit: usize) -> Result<Vec<UsageTotal>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE ids(id) AS (
                 SELECT MIN(app_id) FROM sessions
                 UNION ALL
                 SELECT (SELECT MIN(app_id) FROM sessions WHERE app_id > ids.id)
                 FROM ids WHERE ids.id IS NOT NULL
             ), latest(r) AS (
                 SELECT (SELECT rowid FROM sessions
                         WHERE app_id = ids.id AND deleted_at IS NULL
                         ORDER BY ended_at DESC LIMIT 1)
                 FROM ids WHERE ids.id IS NOT NULL
             )
             SELECT s.app_id, s.app_name, 0
             FROM latest JOIN sessions s ON s.rowid = latest.r
             WHERE s.app_id != ?2
             ORDER BY s.ended_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64, IDLE_APP_ID], |r| {
            Ok(UsageTotal {
                key: r.get(0)?,
                label: r.get(1)?,
                seconds: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Bir uygulamanın pencere başlıklarına göre süre dağılımı (cihazlar arası çakışmalar
    /// [`Self::app_totals`] gibi bir kez sayılır).
    pub fn title_totals(
        &self,
        app_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| {
            (s.app_id == app_id).then_some((&s.title, &s.title))
        })
    }

    /// `key` ile gruplanmış toplam süre. Bilgisayarlar arası çakışmalar bir kez sayılır ve
    /// süreler milisaniye olarak toplanıp en sonda saniyeye çevrilir (oturum başına kırpılmaz).
    pub(super) fn totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        key: impl Fn(&Session) -> Option<(&String, &String)>,
    ) -> Result<Vec<UsageTotal>> {
        let sessions = self.merged_sessions_between(from, to)?;
        // anahtar → (en son görülen ad, milisaniye)
        let mut sums: HashMap<&String, (&String, i64)> = HashMap::new();
        for s in &sessions {
            let Some((k, label)) = key(s) else { continue };
            let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
            if ms <= 0 {
                continue;
            }
            let entry = sums.entry(k).or_insert((label, 0));
            entry.0 = label;
            entry.1 += ms;
        }
        let mut out: Vec<UsageTotal> = sums
            .into_iter()
            .map(|(k, (label, ms))| UsageTotal {
                key: k.clone(),
                label: label.clone(),
                seconds: ms / 1000,
            })
            .collect();
        out.sort_by(|a, b| {
            b.seconds
                .cmp(&a.seconds)
                .then_with(|| a.label.cmp(&b.label))
        });
        Ok(out)
    }
}
