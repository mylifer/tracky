//! Oturumlar: kayıt, aralıkta okuma, atama, silme, takvim bloğunu boyutlandırma ve elle kayıt.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use rusqlite::params;
use uuid::Uuid;

use super::{Result, Store, StoreError, UsageTotal, from_ms, ms};
use crate::classify::{Classifier, TagKind};
use crate::model::{IDLE_APP_ID, MANUAL_APP_ID, Session};

/// `[?1, ?2)` ile kesişen oturumlar. Üçüncü koşul sonucu değiştirmez (kesişen her oturum
/// en uzun oturumdan kısadır), yalnızca indeksin alt sınırıdır.
pub(super) const OVERLAPS: &str = "started_at < ?2 AND ended_at > ?1
    AND started_at >= ?1 - (SELECT max_duration FROM session_stats)";

/// Aralık düzenlemesinin yalnızca bazı uygulamalara (ve isteğe bağlı başlıklarına) uygulanması:
/// uygulama çizelgesinde bir uygulamanın çubuğuna tıklanınca aynı dilimdeki öteki uygulamalar
/// değişmesin.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditScope {
    pub app_ids: Vec<String>,
    /// Doluysa yalnızca bu pencere başlıkları.
    pub titles: Option<Vec<String>>,
}

/// `scope`'un SQL parametreleri (JSON dizileri; kapsam yoksa NULL): [`in_scope`] için.
fn scope_params(scope: Option<&EditScope>) -> Result<(Option<String>, Option<String>)> {
    let Some(scope) = scope else {
        return Ok((None, None));
    };
    if scope.app_ids.is_empty() {
        return Err(StoreError::Invalid("uygulama seçilmedi".into()));
    }
    Ok((
        Some(serde_json::to_string(&scope.app_ids)?),
        scope
            .titles
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
    ))
}

/// `?n` (uygulamalar) ve `?n+1` (başlıklar) parametreleriyle kapsam koşulu.
fn in_scope(n: usize) -> String {
    format!(
        "(?{n} IS NULL OR app_id IN (SELECT value FROM json_each(?{n})))
         AND (?{m} IS NULL OR title IN (SELECT value FROM json_each(?{m})))",
        m = n + 1
    )
}

/// Başka cihazın oturumu, bitişi bu kadar yakın olduğu sürece "sürüyor" sayılır. Cihazlar
/// 5 dakikada bir eşitlediği için buradaki kopya o cihazın gerçek durumundan ~10 dakika
/// geride olabilir.
pub(super) const FOREIGN_LIVE_WINDOW_MS: i64 = 15 * 60 * 1000;

impl Store {
    /// Oturumu ekler ya da (devam eden oturumun periyodik kaydında) günceller.
    pub fn upsert_session(&self, s: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
                (id, device_id, app_id, app_name, title, url, domain, started_at, ended_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET
                title = excluded.title,
                url = excluded.url,
                domain = excluded.domain,
                -- Süren oturum silindiyse (takvimde blok silme, başka cihaz) takip
                -- sürüyor demektir: kayıt silinme anından itibaren yeniden başlar.
                started_at = CASE WHEN sessions.deleted_at IS NULL THEN sessions.started_at
                    ELSE MAX(sessions.started_at, MIN(sessions.deleted_at, excluded.ended_at)) END,
                deleted_at = NULL,
                state_at = CASE WHEN sessions.deleted_at IS NULL THEN sessions.state_at
                    ELSE excluded.updated_at END,
                ended_at = excluded.ended_at,
                updated_at = MAX(excluded.updated_at, sessions.updated_at + 1)",
            params![
                s.id.to_string(),
                self.device_id.to_string(),
                s.app_id,
                s.app_name,
                s.title,
                s.url,
                s.domain,
                ms(s.started_at),
                ms(s.ended_at),
                ms(Utc::now()),
            ],
        )?;
        Ok(())
    }

    /// Toplamlar için: `[from, to)` ile kesişen çalışma oturumları, bilgisayarlar arası
    /// çakışmalar bir kez sayılacak şekilde birleştirilmiş ([`crate::model::merge_devices`]).
    /// Atanmamış boşta kayıtları dahil değildir ([`Session::counts_as_work`]).
    pub fn merged_sessions_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Session>> {
        let mut sessions = self.merged_sessions_with_idle_between(from, to)?;
        sessions.retain(Session::counts_as_work);
        Ok(sessions)
    }

    /// Takvim için: [`Self::merged_sessions_between`] gibi, ama boşta kayıtlarıyla.
    pub fn merged_sessions_with_idle_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Session>> {
        Ok(crate::model::merge_devices(
            self.sessions_between(from, to)?,
        ))
    }

    /// `[from, to)` ile kesişen oturumlar (ham, cihazlar üst üste binebilir), başlangıca göre sıralı.
    pub fn sessions_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Session>> {
        Ok(self
            .sessions_with_devices_between(from, to)?
            .into_iter()
            .map(|(s, _)| s)
            .collect())
    }

    /// [`Self::sessions_between`], her oturumun bilgisayarıyla.
    pub(super) fn sessions_with_devices_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(Session, String)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, app_id, app_name, title, url, domain, started_at, ended_at, category_id,
                    project_id, device_id, block_from
             FROM sessions
             WHERE deleted_at IS NULL AND {OVERLAPS}
             ORDER BY started_at"
        ))?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Session {
                    id: Uuid::nil(),
                    app_id: r.get(1)?,
                    app_name: r.get(2)?,
                    title: r.get(3)?,
                    url: r.get(4)?,
                    domain: r.get(5)?,
                    started_at: from_ms(r.get(6)?),
                    ended_at: from_ms(r.get(7)?),
                    category_id: r.get(8)?,
                    project_id: r.get(9)?,
                    block_from: r.get::<_, Option<i64>>(11)?.map(from_ms),
                },
                r.get::<_, String>(10)?,
            ))
        })?;
        rows.map(|row| {
            let (id, mut s, device) = row?;
            s.id = Uuid::parse_str(&id).map_err(|e| StoreError::Invalid(e.to_string()))?;
            Ok((s, device))
        })
        .collect()
    }

    /// Uygulama başına toplam süre; aralık dışına taşan kısımlar kırpılır.
    pub fn app_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| Some((&s.app_id, &s.app_name)))
    }

    /// Domain başına toplam süre (yalnızca URL'si bilinen oturumlar).
    pub fn domain_totals(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<UsageTotal>> {
        self.totals(from, to, |s| s.domain.as_ref().map(|d| (d, d)))
    }

    /// Oturumu yumuşak siler (örn. boşta kalma sonrası geçersiz kalan kayıt).
    pub fn delete_session(&self, id: &Uuid) -> Result<()> {
        let now = ms(Utc::now());
        self.conn.execute(
            "UPDATE sessions SET deleted_at = ?2, state_at = ?2, updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id.to_string(), now],
        )?;
        Ok(())
    }

    /// `[from, to)` ile kesişen oturumlara elle kategori verir (`None`: kurallara
    /// dön). Takvimdeki bir blok bu oturumlardan oluşur. Değişen satır sayısı.
    pub fn set_category_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
    ) -> Result<usize> {
        self.set_category_in(from, to, category_id, None)
    }

    /// [`Self::set_category_between`]; `scope` verilirse yalnızca o uygulamaların (ve
    /// başlıkların) oturumları.
    pub fn set_category_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        if let Some(id) = category_id {
            self.require_tag(id, TagKind::Category)?;
        }
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET category_id = ?3, state_at = ?4, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND category_id IS NOT ?3 AND {scope}",
                scope = in_scope(5)
            ),
            params![ms(from), ms(to), category_id, ms(Utc::now()), apps, titles],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` aralığındaki oturumlara elle proje verir (sınırda bölünür);
    /// `None` kurallara döndürür, [`crate::classify::NO_PROJECT`] projesiz yapar. Değişen
    /// satır sayısı.
    pub fn set_project_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        self.set_project_in(from, to, project_id, None)
    }

    /// [`Self::set_project_between`]; `scope` verilirse yalnızca o uygulamaların (ve
    /// başlıkların) oturumları.
    pub fn set_project_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        project_id: Option<&str>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        if let Some(id) = project_id.filter(|id| *id != crate::classify::NO_PROJECT) {
            self.require_tag(id, TagKind::Project)?;
        }
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET project_id = ?3, state_at = ?4, updated_at = MAX(?4, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND project_id IS NOT ?3 AND {scope}",
                scope = in_scope(5)
            ),
            params![ms(from), ms(to), project_id, ms(Utc::now()), apps, titles],
        )?;
        // Raporda bilerek atanan süre, projenin silinmiş satırında kalsa da yeniden önerilir.
        if let Some(id) = project_id.filter(|id| *id != crate::classify::NO_PROJECT) {
            self.forget_dismissed(id, from, to)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` içindeki süreyi yumuşak siler; sınırı aşan oturumların dışarıda
    /// kalan kısmı korunur. Silinen satır sayısı.
    pub fn delete_between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        self.delete_in(from, to, None)
    }

    /// [`Self::delete_between`]; `scope` verilirse yalnızca o uygulamaların (ve başlıkların)
    /// oturumları.
    pub fn delete_in(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        scope: Option<&EditScope>,
    ) -> Result<usize> {
        self.ensure_no_foreign_live(from, to)?;
        let (apps, titles) = scope_params(scope)?;
        let tx = self.savepoint()?;
        self.split_at(from, to)?;
        let n = self.conn.execute(
            &format!(
                "UPDATE sessions SET deleted_at = ?3, state_at = ?3, updated_at = MAX(?3, updated_at + 1)
                 WHERE deleted_at IS NULL AND {OVERLAPS} AND {scope}",
                scope = in_scope(4)
            ),
            params![ms(from), ms(to), ms(Utc::now()), apps, titles],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// Takvim bloğunu `[from, to)` aralığından `[new_from, new_to)` aralığına uzatır ya da
    /// kısaltır. Kısaltınca blok ikiye bölünür: dışarıda kalan kısım silinmez, kategorisi ve
    /// projesiyle ayrı blok (ayrı çizelge satırı) olur ([`Session::block_from`]). Bloğa
    /// katılan kısımdaki kayıtlar bloğun kategorisini ve projesini alır, aradaki bölmeler
    /// kalkar; kaydı olmayan boşluklar (bilgisayar başında olunmayan süre) aynı kategori ve
    /// projede `label` adlı elle kayıtla dolar: zaman çizelgesine bloğun yeni aralığı gider.
    /// Projesi olmayan blok projeli işin üstüne uzarsa o işin projesini alır: iki blok tek
    /// blok (tek çizelge satırı) olur. Değişen satır sayısı.
    #[allow(clippy::too_many_arguments)]
    pub fn resize_block(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        new_from: DateTime<Utc>,
        new_to: DateTime<Utc>,
        label: &str,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        if new_to <= new_from {
            return Err(StoreError::Invalid(
                "bitiş başlangıçtan sonra olmalı".into(),
            ));
        }
        if new_to > Utc::now() {
            return Err(StoreError::Invalid(
                "blok henüz gelmemiş bir zamana uzatılamaz".into(),
            ));
        }
        let joined = [(new_from, from.min(new_to)), (to.max(new_from), new_to)];
        let adopted = match project_id {
            Some(_) => None,
            None => self.joined_project(&joined)?,
        };
        let project_id = project_id.or(adopted.as_deref());
        let tx = self.savepoint()?;
        let mut n = 0;
        if new_from < from || new_to > to {
            n += self.clear_block_starts(new_from, new_to)?;
        }
        // Kesilen baştan sonra kalan blok, kesilen sondan sonra kesilen parça yeni blok başlatır.
        if new_from > from && new_from < to {
            n += self.mark_block_start(new_from, to.min(new_to))?;
        }
        if new_to < to && new_to > from {
            n += self.mark_block_start(new_to.max(new_from), to)?;
        }
        for (a, b) in joined {
            if b > a {
                n += self.join_block(a, b, label, category_id, project_id)?;
            }
        }
        // Bloğun kendi kısmı da katıldığı işin projesine geçer.
        let (a, b) = (from.max(new_from), to.min(new_to));
        if adopted.is_some() && b > a {
            n += self.set_project_between(a, b, project_id)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// `[from, to)` içinde başlayan ilk çalışma oturumundan yeni takvim bloğu başlatır
    /// ([`Session::block_from`]); `from`'u aşan oturum önce bölünür. Değişen satır sayısı.
    fn mark_block_start(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        self.ensure_no_foreign_live(from, to)?;
        self.split_at(from, to)?;
        let Some(first) = self
            .sessions_between(from, to)?
            .into_iter()
            .find(|s| s.started_at >= from && s.counts_as_work())
        else {
            return Ok(0);
        };
        let now = ms(Utc::now());
        Ok(self.conn.execute(
            "UPDATE sessions SET block_from = started_at, state_at = ?2,
                 updated_at = MAX(?2, updated_at + 1)
             WHERE id = ?1",
            params![first.id.to_string(), now],
        )?)
    }

    /// `(from, to)` içindeki elle bölmeleri kaldırır (aralığın başındaki kalır): aralık tek
    /// blok olur. Değişen satır sayısı.
    fn clear_block_starts(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<usize> {
        let now = ms(Utc::now());
        Ok(self.conn.execute(
            "UPDATE sessions SET block_from = NULL, state_at = ?3,
                 updated_at = MAX(?3, updated_at + 1)
             WHERE deleted_at IS NULL AND block_from > ?1 AND block_from < ?2",
            params![ms(from), ms(to), now],
        )?)
    }

    /// [`Self::resize_block`]'ta projesi olmayan bloğa katılan aralıklarda en çok süren proje
    /// ("Projesiz" sayılmaz).
    fn joined_project(&self, ranges: &[(DateTime<Utc>, DateTime<Utc>)]) -> Result<Option<String>> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let mut by_project: HashMap<String, i64> = HashMap::new();
        for &(from, to) in ranges.iter().filter(|(a, b)| b > a) {
            for s in self.sessions_between(from, to)? {
                if !s.counts_as_work() {
                    continue;
                }
                let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
                if let (Some(p), true) = (classifier.classify(&s).project, ms > 0) {
                    *by_project.entry(p).or_default() += ms;
                }
            }
        }
        Ok(by_project
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(p, _)| p))
    }

    /// [`Self::resize_block`]'ta bloğa katılan `[from, to)` aralığı.
    fn join_block(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        label: &str,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<usize> {
        let mut n = 0;
        if category_id.is_some() {
            n += self.set_category_between(from, to, category_id)?;
        }
        if project_id.is_some() {
            n += self.set_project_between(from, to, project_id)?;
        }
        // Atamadan sonra hâlâ çalışma sayılmayan (kaydı olmayan ya da boşta) kısımlar.
        let mut busy: Vec<_> = self
            .sessions_between(from, to)?
            .into_iter()
            .filter(Session::counts_as_work)
            .map(|s| (s.started_at.max(from), s.ended_at.min(to)))
            .collect();
        busy.sort();
        busy.push((to, to));
        let manual_project = project_id.filter(|p| *p != crate::classify::NO_PROJECT);
        let mut at = from;
        for (a, b) in busy {
            if a - at >= Duration::minutes(1) {
                self.add_manual_session(label, at, a, category_id, manual_project)?;
                n += 1;
            }
            at = at.max(b);
        }
        Ok(n)
    }

    /// Aralıkta başka cihazın hâlâ sürüyor olabilecek oturumu varsa düzenlemeyi reddeder.
    /// O cihaz süren oturumu birkaç saniyede bir yeniden kaydeder; satırın tamamı son
    /// yazanla eşitlendiği için buradaki bölme, silme ya da atama geri alınırdı.
    /// Elle eklenen kayıtlar ve boşta kayıtları sürmez (bitince bir kez yazılır), kapsam dışıdır.
    fn ensure_no_foreign_live(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        let live: bool = self.conn.query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM sessions
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND device_id != ?3 AND ended_at > ?4
                   AND substr(app_id, 1, length(?5)) != ?5 AND app_id != ?6)"
            ),
            params![
                ms(from),
                ms(to),
                self.device_id.to_string(),
                ms(Utc::now()) - FOREIGN_LIVE_WINDOW_MS,
                format!("{MANUAL_APP_ID}/"),
                IDLE_APP_ID,
            ],
            |r| r.get(0),
        )?;
        if live {
            return Err(StoreError::ForeignLiveSession);
        }
        Ok(())
    }

    /// `[from, to)` sınırını aşan oturumları sınırlarda böler; sonra aralıkla
    /// kesişen her oturum tamamen aralığın içindedir. Asıl kimlik en son parçada
    /// kalır: süren oturumu takip eden motor doğru satırı uzatmaya devam eder.
    fn split_at(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        // Boş aralık sıfır uzunlukta parça, ters aralık ham bir CHECK hatası üretirdi.
        if to <= from {
            return Err(StoreError::Invalid(
                "aralığın sonu başlangıcından sonra olmalı".into(),
            ));
        }
        let (from, to, now) = (ms(from), ms(to), ms(Utc::now()));
        let partial: Vec<(String, i64, i64)> = self
            .conn
            .prepare(&format!(
                "SELECT id, started_at, ended_at FROM sessions
                 WHERE deleted_at IS NULL AND {OVERLAPS}
                   AND (started_at < ?1 OR ended_at > ?2)"
            ))?
            .query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, start, end) in partial {
            let mut parts = Vec::new();
            if start < from {
                parts.push((start, from));
            }
            parts.push((start.max(from), end.min(to)));
            if end > to {
                parts.push((to, end));
            }
            let (keep_start, keep_end) = parts.pop().expect("en az bir parça");
            for (a, b) in parts {
                self.conn.execute(
                    "INSERT INTO sessions (id, device_id, app_id, app_name, title, url, domain,
                         category_id, project_id, state_at, block_from, started_at, ended_at, updated_at)
                     SELECT ?2, device_id, app_id, app_name, title, url, domain, category_id,
                         project_id, state_at, block_from, ?3, ?4, ?5
                     FROM sessions WHERE id = ?1",
                    params![id, Uuid::new_v4().to_string(), a, b, now],
                )?;
            }
            self.conn.execute(
                "UPDATE sessions SET started_at = ?2, ended_at = ?3, updated_at = MAX(?4, updated_at + 1)
                 WHERE id = ?1",
                params![id, keep_start, keep_end, now],
            )?;
        }
        Ok(())
    }

    /// Elle kayıt ekler (bilgisayar dışında geçen toplantı, okuma...).
    pub fn add_manual_session(
        &self,
        label: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        category_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Session> {
        let label = label.trim();
        if label.is_empty() {
            return Err(StoreError::Invalid("kayıt adı boş".into()));
        }
        if to <= from {
            return Err(StoreError::Invalid(
                "bitiş başlangıçtan sonra olmalı".into(),
            ));
        }
        if let Some(id) = category_id {
            self.require_tag(id, TagKind::Category)?;
        }
        if let Some(id) = project_id {
            self.require_tag(id, TagKind::Project)?;
        }
        // Takip edilen süreyle çakışırsa aynı dakikalar iki kez sayılırdı. Atanmamış boşta
        // kaydının yerini ise elle kayıt alır (boşta geçen süreyi açıklamanın yolu bu).
        let overlapping = self.sessions_between(from, to)?;
        if overlapping.iter().any(Session::counts_as_work) {
            return Err(StoreError::Invalid(
                "bu aralıkta zaten kayıt var; önce o bloğu silin".into(),
            ));
        }
        let tx = self.savepoint()?;
        if !overlapping.is_empty() {
            self.split_at(from, to)?;
            self.conn.execute(
                &format!(
                    "UPDATE sessions SET deleted_at = ?3, state_at = ?3, updated_at = MAX(?3, updated_at + 1)
                     WHERE deleted_at IS NULL AND {OVERLAPS} AND app_id = ?4"
                ),
                params![ms(from), ms(to), ms(Utc::now()), IDLE_APP_ID],
            )?;
        }
        let session = Session {
            id: Uuid::new_v4(),
            app_id: format!("{MANUAL_APP_ID}/{label}"),
            app_name: label.into(),
            title: label.into(),
            url: None,
            domain: None,
            started_at: from,
            ended_at: to,
            category_id: category_id.map(Into::into),
            project_id: project_id.map(Into::into),
            block_from: None,
        };
        self.upsert_session(&session)?;
        self.conn.execute(
            "UPDATE sessions SET category_id = ?2, project_id = ?3 WHERE id = ?1",
            params![
                session.id.to_string(),
                session.category_id,
                session.project_id
            ],
        )?;
        tx.commit()?;
        Ok(session)
    }
}
