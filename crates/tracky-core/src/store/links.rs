//! Takvim düzeltmelerinden öğrenilen tür çiftleri ([`crate::blocks::Links`]): kullanıcı bir bloğu
//! iki türün arasından bölünce (kısaltınca) `split`, bir türün bloğunu ötekinin üstüne uzatınca
//! `join` kaydı düşer ([`Store::resize_block`]). Her tür çiftinin hükmü düzeltmelerin sayısından
//! ve sonuncusunun yönünden çıkar. Kayıtlar eşitlenir (silme yumuşaktır).

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use rusqlite::params;
use uuid::Uuid;

use super::{Result, Store, ms};
use crate::blocks::{CONTEXT_MIN, LEARNED_MIN, Link, Links, context_key};
use crate::classify::{Classifier, Tag};

impl Store {
    /// Etiketler, kurallar ve öğrenilen tür çiftleriyle sınıflandırıcı.
    pub(crate) fn classifier(&self) -> Result<Classifier> {
        self.classifier_for(&self.tags()?)
    }

    /// [`Self::classifier`]; etiketler zaten okunmuşsa.
    pub(crate) fn classifier_for(&self, tags: &[Tag]) -> Result<Classifier> {
        Ok(Classifier::new(tags, &self.rules()?).with_links(self.block_links()?))
    }

    /// Öğrenilen tür çiftleri.
    pub fn block_links(&self) -> Result<Links> {
        let mut stmt = self.conn.prepare(
            "SELECT a, b, kind FROM block_links WHERE deleted_at IS NULL
             ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut signals = Vec::new();
        for row in rows {
            let (a, b, kind) = row?;
            if let Some(link) = Link::parse(&kind) {
                signals.push((a, b, link));
            }
        }
        Ok(Links::from_signals(signals))
    }

    /// `[from, to)` aralığındaki bloğun düzeltmesini kaydeder (aynı türse kaydedilmez).
    pub(crate) fn record_block_link(
        &self,
        (a, b, link): (&str, &str, Link),
        (from, to): (DateTime<Utc>, DateTime<Utc>),
    ) -> Result<()> {
        if a == b {
            return Ok(());
        }
        let now = ms(Utc::now());
        self.conn.execute(
            "INSERT INTO block_links (id, a, b, kind, started_at, ended_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                Uuid::new_v4().to_string(),
                a,
                b,
                link.as_str(),
                ms(from),
                ms(to),
                now
            ],
        )?;
        Ok(())
    }

    /// `at`'ten (ms) sonra `[start, end)` ile kesişen bloklara düşen düzeltmeleri siler
    /// (düzenleme geri alınınca; [`Self::restore_snapshot`]).
    pub(super) fn forget_block_links(&self, at: i64, (start, end): (i64, i64)) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "UPDATE block_links SET deleted_at = ?4, updated_at = MAX(?4, updated_at + 1)
             WHERE deleted_at IS NULL AND created_at >= ?1 AND started_at < ?3 AND ended_at > ?2",
            params![at, start, end, now],
        )?;
        Ok(())
    }

    /// [`Self::resize_block`]'ın öğrettikleri, düzenlemeden önce bakılarak. Kısaltılan kenarda
    /// kesim anının hemen önündeki ve ardındaki (`CONTEXT_MIN` içinde) baskın türler farklıysa
    /// `split`; uzatılan kenarda katılan kısımda en az `LEARNED_MIN` süren, bloğun baskın
    /// türünden farklı her tür için `join`.
    pub(super) fn resize_links(
        &self,
        (from, to): (DateTime<Utc>, DateTime<Utc>),
        (new_from, new_to): (DateTime<Utc>, DateTime<Utc>),
    ) -> Result<Vec<(String, String, Link)>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut out = Vec::new();
        for cut in [new_from, new_to] {
            if cut <= from || cut >= to {
                continue;
            }
            let before = (cut - CONTEXT_MIN).max(from);
            let after = (cut + CONTEXT_MIN).min(to);
            let dominant = |range| -> Result<Option<(String, Duration)>> {
                Ok(self
                    .contexts_in(&classifier, range)?
                    .into_iter()
                    .next()
                    .filter(|(_, d)| *d >= LEARNED_MIN))
            };
            if let (Some((a, _)), Some((b, _))) =
                (dominant((before, cut))?, dominant((cut, after))?)
                && a != b
            {
                out.push((a, b, Link::Split));
            }
        }
        let kept = (from.max(new_from), to.min(new_to));
        let Some((block, _)) = self.contexts_in(&classifier, kept)?.into_iter().next() else {
            return Ok(out);
        };
        for range in [(new_from, from), (to, new_to)] {
            if range.1 <= range.0 {
                continue;
            }
            for (other, work) in self.contexts_in(&classifier, range)? {
                if work >= LEARNED_MIN && other != block {
                    out.push((block.clone(), other, Link::Join));
                }
            }
        }
        Ok(out)
    }

    /// Aralıktaki işin türleri ([`context_key`]) ve süreleri, çoktan aza.
    fn contexts_in(
        &self,
        classifier: &Classifier,
        (from, to): (DateTime<Utc>, DateTime<Utc>),
    ) -> Result<Vec<(String, Duration)>> {
        if to <= from {
            return Ok(Vec::new());
        }
        let mut by: HashMap<String, i64> = HashMap::new();
        for s in self.sessions_between(from, to)? {
            if !s.counts_as_work() {
                continue;
            }
            let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
            if ms <= 0 {
                continue;
            }
            let class = classifier.classify(&s);
            *by.entry(context_key(class.category.as_deref(), &s.app_name))
                .or_default() += ms;
        }
        let mut list: Vec<(String, i64)> = by.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(list
            .into_iter()
            .map(|(k, ms)| (k, Duration::milliseconds(ms)))
            .collect())
    }
}
