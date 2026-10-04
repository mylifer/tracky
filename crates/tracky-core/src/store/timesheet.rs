//! Zaman çizelgesi: ayarlar, toplantı atamaları, günlük kayıt önerileri ve onaylanmış kayıtlar.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{Result, Store, StoreError, from_ms, ms};
use crate::classify::{Classifier, TagKind};
use crate::meeting_suggest::{MeetingSuggester, ProjectInfo, SuggestInput};
use crate::model::Session;
use crate::timesheet::{self, EntryKind, Meeting, MeetingProject, TimesheetConfig, TimesheetEntry};

/// Toplantı önerilerinde projelerin kullanımına bakılan dönem (gün; zaman çizelgesi kayıtları).
const SUGGEST_USAGE_DAYS: u64 = 90;

/// Zaman çizelgesi ayarları.
const TIMESHEET_KEY: &str = "timesheet";
/// Takvim toplantı serilerinin elle verilen projesi (UID → proje; `null`: yoksay).
const MEETING_ASSIGNMENTS_KEY: &str = "meeting_assignments";

/// (Projesi belli toplantılar ve projeleri, hiçbir projeye düşmeyen toplantılar).
pub type SplitMeetings = (Vec<(Meeting, String)>, Vec<Meeting>);

/// Onaylanmış zaman çizelgesi kaydı.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEntry {
    pub id: String,
    #[serde(flatten)]
    pub entry: TimesheetEntry,
    /// Excel'e aktarıldığı an; doluysa kayıt değiştirilemez.
    pub exported_at: Option<DateTime<Utc>>,
}

fn parse_kind(s: &str) -> Option<EntryKind> {
    match s {
        "Working" => Some(EntryKind::Working),
        "Online" => Some(EntryKind::Online),
        "F2F" => Some(EntryKind::F2F),
        _ => None,
    }
}

impl Store {
    pub fn timesheet_config(&self) -> Result<TimesheetConfig> {
        Ok(self.setting(TIMESHEET_KEY)?.unwrap_or_default())
    }

    pub fn save_timesheet_config(&self, config: &TimesheetConfig) -> Result<()> {
        self.save_setting(TIMESHEET_KEY, config)
    }

    /// Toplantı serilerinin elle verilen projeleri (UID → proje; `None`: yoksayıldı).
    pub fn meeting_assignments(&self) -> Result<HashMap<String, Option<String>>> {
        Ok(self.setting(MEETING_ASSIGNMENTS_KEY)?.unwrap_or_default())
    }

    /// Toplantı serisini projeye atar (`None`: zaman çizelgesine alma).
    pub fn assign_meeting(&self, uid: &str, project: Option<&str>) -> Result<()> {
        let mut all = self.meeting_assignments()?;
        all.insert(uid.to_string(), project.map(str::to_string));
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)
    }

    /// Yoksayılan toplantıları geri getirir; sayısını döndürür.
    pub fn restore_ignored_meetings(&self) -> Result<usize> {
        let mut all = self.meeting_assignments()?;
        let before = all.len();
        all.retain(|_, p| p.is_some());
        self.save_setting(MEETING_ASSIGNMENTS_KEY, &all)?;
        Ok(before - all.len())
    }

    /// Toplantıları projesine göre ayırır: (projesi belli olanlar, hiçbir projeye düşmeyenler).
    pub fn classify_meetings(&self, meetings: &[Meeting]) -> Result<SplitMeetings> {
        let classifier = Classifier::new(&self.tags()?, &self.rules()?);
        let assigned = self.meeting_assignments()?;
        let (mut known, mut unassigned) = (Vec::new(), Vec::new());
        for m in meetings {
            match timesheet::meeting_project(m, &classifier, &assigned) {
                MeetingProject::Project(p) => known.push((m.clone(), p)),
                MeetingProject::Unassigned => unassigned.push(m.clone()),
                MeetingProject::Ignored => {}
            }
        }
        Ok((known, unassigned))
    }

    /// Toplantı → proje öneri modeli ([`crate::meeting_suggest`]). `series` takvimdeki her
    /// seriden bir örnektir ([`crate::calendar::Calendar::series`]); projesi belli olanlar
    /// (elle atanan ya da kurala uyan) geçmiş olur. Müşterinin projeleri arasında son
    /// [`SUGGEST_USAGE_DAYS`] günün zaman çizelgesi saatlerine göre seçilir.
    pub fn meeting_suggester(&self, series: &[Meeting]) -> Result<MeetingSuggester> {
        let tags = self.tags()?;
        let rules = self.rules()?;
        let classifier = Classifier::new(&tags, &rules);
        let assigned = self.meeting_assignments()?;
        let history: Vec<(Meeting, String)> = series
            .iter()
            .filter_map(
                |m| match timesheet::meeting_project(m, &classifier, &assigned) {
                    MeetingProject::Project(p) => Some((m.clone(), p)),
                    _ => None,
                },
            )
            .collect();
        let client_names: HashMap<String, String> = self
            .clients()?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let project_clients = self.project_clients()?;
        let archived = self.archived_projects()?;
        let projects: Vec<ProjectInfo> = tags
            .into_iter()
            .filter(|t| t.kind == TagKind::Project)
            .map(|t| ProjectInfo {
                client: project_clients
                    .get(&t.id)
                    .and_then(|c| client_names.get(c))
                    .cloned(),
                archived: archived.contains(&t.id),
                id: t.id,
                name: t.name,
            })
            .collect();
        let today = chrono::Local::now().date_naive();
        let mut usage: HashMap<String, f64> = HashMap::new();
        for e in self.timesheet_entries(today - chrono::Days::new(SUGGEST_USAGE_DAYS), today)? {
            *usage.entry(e.entry.project_id).or_default() += e.entry.hours;
        }
        Ok(MeetingSuggester::new(&SuggestInput {
            projects: &projects,
            history: &history,
            all: series,
            rules: &rules,
            usage: &usage,
        }))
    }

    /// Günün (`day_start`–`day_end`, yerel gün) oturumlarından ve takvim toplantılarından
    /// iş kaydı önerileri.
    pub fn propose_timesheet(
        &self,
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        self.propose_from(
            &self.merged_sessions_between(day_start, day_end)?,
            day_start,
            day_end,
            meetings,
        )
    }

    /// Yalnızca toplantılardan iş kaydı önerileri (takip edilen süre olmadan).
    pub fn propose_meetings(
        &self,
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        self.propose_from(&[], day_start, day_end, meetings)
    }

    fn propose_from(
        &self,
        sessions: &[Session],
        day_start: DateTime<Utc>,
        day_end: DateTime<Utc>,
        meetings: &[Meeting],
    ) -> Result<Vec<TimesheetEntry>> {
        let tags = self.tags()?;
        let classifier = Classifier::new(&tags, &self.rules()?);
        let names = tags
            .iter()
            .filter(|t| t.kind == TagKind::Project)
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();
        let (meetings, _) = self.classify_meetings(meetings)?;
        Ok(timesheet::propose(
            sessions,
            &meetings,
            &classifier,
            &names,
            &self.timesheet_config()?,
            day_start,
            day_end,
        ))
    }

    /// `[from, to]` tarihleri (dahil) arasındaki onaylanmış kayıtlar, tarih ve saate göre.
    pub fn timesheet_entries(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<SavedEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, date, start, hours, kind, details, party, project_id, division, exported_at,
                    actual_hours
             FROM timesheet_entries WHERE date >= ?1 AND date <= ?2 ORDER BY date, start",
        )?;
        let rows = stmt.query_map(params![from.to_string(), to.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<f64>>(10)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                date,
                start,
                hours,
                kind,
                details,
                party,
                project_id,
                division,
                exported,
                actual,
            ) = row?;
            let bad = |what: &str| StoreError::Invalid(format!("zaman çizelgesi {what}: {id}"));
            Ok(SavedEntry {
                entry: TimesheetEntry {
                    date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                        .map_err(|_| bad("tarihi"))?,
                    start: NaiveTime::parse_from_str(&start, "%H:%M").map_err(|_| bad("saati"))?,
                    hours,
                    actual_hours: actual,
                    kind: parse_kind(&kind).ok_or_else(|| bad("türü"))?,
                    details,
                    party,
                    project_id,
                    division,
                },
                exported_at: exported.map(from_ms),
                id,
            })
        })
        .collect()
    }

    /// Günün aktarılmamış kayıtlarını verilenlerle değiştirir (onaylama, yeniden öneri).
    /// Excel'e aktarılmış kayıtlara dokunulmaz ve aktarılan iş yeniden eklenmez
    /// ([`timesheet::without_exported`]); yoksa bir sonraki aktarımda dosyaya iki kez yazılırdı.
    pub fn replace_timesheet_day(&self, date: NaiveDate, entries: &[TimesheetEntry]) -> Result<()> {
        let tx = self.savepoint()?;
        self.conn.execute(
            "DELETE FROM timesheet_entries WHERE date = ?1 AND exported_at IS NULL",
            [date.to_string()],
        )?;
        let exported: Vec<TimesheetEntry> = self
            .timesheet_entries(date, date)?
            .into_iter()
            .map(|e| e.entry)
            .collect();
        for e in timesheet::without_exported(entries, &exported) {
            self.insert_entry(&Uuid::new_v4().to_string(), &e)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Kaydı ekler ya da (aynı kimlikle) günceller; aktarılmış kayıt değiştirilemez.
    pub fn save_timesheet_entry(&self, id: Option<&str>, entry: &TimesheetEntry) -> Result<String> {
        if !(entry.hours > 0.0 && entry.hours <= 24.0) {
            return Err(StoreError::Invalid("saat 0 ile 24 arasında olmalı".into()));
        }
        let id = id.map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
        let exported: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT exported_at FROM timesheet_entries WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        if matches!(exported, Some(Some(_))) {
            return Err(StoreError::Invalid(
                "Excel'e aktarılmış kayıt değiştirilemez".into(),
            ));
        }
        self.conn
            .execute("DELETE FROM timesheet_entries WHERE id = ?1", [&id])?;
        self.insert_entry(&id, entry)?;
        Ok(id)
    }

    /// Onaylı güne eklenen toplantı satırlarını değiştirir: `old` (toplantının önceki
    /// atamasıyla eklenen satırlar) ile birebir aynı, aktarılmamış kayıtları siler ve `new`'u
    /// ekler. Elle değiştirilen ya da aktarılmış satırlara dokunulmaz.
    pub fn replace_meeting_entries(
        &self,
        date: NaiveDate,
        old: &[TimesheetEntry],
        new: &[TimesheetEntry],
    ) -> Result<()> {
        let tx = self.savepoint()?;
        let mut saved: Vec<SavedEntry> = self
            .timesheet_entries(date, date)?
            .into_iter()
            .filter(|s| s.exported_at.is_none())
            .collect();
        for entry in old {
            let same = |s: &SavedEntry| {
                let e = &s.entry;
                e.date == entry.date
                    && e.start.format("%H:%M").to_string()
                        == entry.start.format("%H:%M").to_string()
                    && e.hours == entry.hours
                    && e.kind == entry.kind
                    && e.project_id == entry.project_id
                    && e.details == entry.details.trim()
                    && e.division == entry.division.trim()
                    && e.party == entry.party.trim()
            };
            if let Some(i) = saved.iter().position(same) {
                let s = saved.remove(i);
                self.conn
                    .execute("DELETE FROM timesheet_entries WHERE id = ?1", [&s.id])?;
            }
        }
        for entry in new {
            self.insert_entry(&Uuid::new_v4().to_string(), entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_timesheet_entry(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM timesheet_entries WHERE id = ?1 AND exported_at IS NULL",
            [id],
        )?;
        Ok(())
    }

    /// Excel'e aktarılan kayıtları işaretler.
    pub fn mark_timesheet_exported(&self, ids: &[String], at: DateTime<Utc>) -> Result<()> {
        let tx = self.savepoint()?;
        for id in ids {
            self.conn.execute(
                "UPDATE timesheet_entries SET exported_at = ?2 WHERE id = ?1",
                params![id, ms(at)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn insert_entry(&self, id: &str, e: &TimesheetEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO timesheet_entries
                (id, date, start, hours, kind, details, party, project_id, division, created_at,
                 actual_hours)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                e.date.to_string(),
                e.start.format("%H:%M").to_string(),
                e.hours,
                e.kind.label(),
                e.details.trim(),
                e.party.trim(),
                e.project_id,
                e.division.trim(),
                ms(Utc::now()),
                e.actual_hours,
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting_row(project: &str, hours: f64) -> TimesheetEntry {
        TimesheetEntry {
            date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            start: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            hours,
            actual_hours: Some(hours),
            kind: EntryKind::Online,
            details: "Haftalık toplantı".into(),
            party: "Togg".into(),
            project_id: project.into(),
            division: project.into(),
        }
    }

    #[test]
    fn reassigning_a_meeting_replaces_its_rows() {
        let store = Store::open_in_memory().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let a = meeting_row("A", 1.0);
        let b = meeting_row("B", 1.0);
        // Onaylı gün: başka bir iş ve A'ya atanmış toplantı.
        let mut other = meeting_row("A", 2.0);
        other.start = NaiveTime::from_hms_opt(13, 0, 0).unwrap();
        other.kind = EntryKind::Working;
        store.replace_timesheet_day(date, &[other.clone()]).unwrap();
        store
            .replace_meeting_entries(date, &[], std::slice::from_ref(&a))
            .unwrap();

        // A → B: A'nın satırı gider, B'ninki gelir; başka iş kalır.
        store
            .replace_meeting_entries(date, std::slice::from_ref(&a), std::slice::from_ref(&b))
            .unwrap();
        let rows: Vec<TimesheetEntry> = store
            .timesheet_entries(date, date)
            .unwrap()
            .into_iter()
            .map(|s| s.entry)
            .collect();
        assert_eq!(rows, vec![b.clone(), other.clone()]);

        // B → yoksay: toplantı satırı kalmaz.
        store.replace_meeting_entries(date, &[b], &[]).unwrap();
        let rows = store.timesheet_entries(date, date).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry, other);
    }

    #[test]
    fn exported_and_edited_meeting_rows_are_kept() {
        let store = Store::open_in_memory().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let a = meeting_row("A", 1.0);
        store
            .replace_meeting_entries(date, &[], std::slice::from_ref(&a))
            .unwrap();
        let id = store.timesheet_entries(date, date).unwrap()[0].id.clone();
        store.mark_timesheet_exported(&[id], Utc::now()).unwrap();
        let edited = TimesheetEntry {
            details: "Elle yazıldı".into(),
            ..a.clone()
        };
        store.save_timesheet_entry(None, &edited).unwrap();
        store.replace_meeting_entries(date, &[a], &[]).unwrap();
        assert_eq!(store.timesheet_entries(date, date).unwrap().len(), 2);
    }

    #[test]
    fn meeting_suggester_learns_from_assigned_series_and_skips_archived() {
        let store = Store::open_in_memory().unwrap();
        let tag = |id: &str, name: &str| crate::classify::Tag {
            id: id.into(),
            kind: TagKind::Project,
            name: name.into(),
            color: 1,
        };
        store.upsert_tag(&tag("p1", "Portal"), 0).unwrap();
        let series = |uid: &str, subject: &str| Meeting {
            uid: uid.into(),
            start: chrono::Utc::now(),
            end: chrono::Utc::now() + chrono::Duration::hours(1),
            subject: subject.into(),
            attendees: vec!["me@kum.dev".into(), "ali@acme.com".into()],
            ..Meeting::default()
        };
        // İç toplantı: kullanıcının kendi alan adı en sık geçen olur.
        let mut internal = series("c", "Ekip");
        internal.attendees.truncate(1);
        let all = [
            series("a", "Planlama"),
            series("b", "Retro"),
            internal,
            series("new", "Yeni konu"),
        ];
        store.assign_meeting("a", Some("p1")).unwrap();
        store.assign_meeting("b", Some("p1")).unwrap();
        let got = store
            .meeting_suggester(&all)
            .unwrap()
            .suggest(&all[3])
            .unwrap();
        assert_eq!(got.project_id, "p1");
        assert_eq!(got.reason, "katılımcılar @acme.com");
        // Arşivlenen proje önerilmez.
        store.archive_project("p1").unwrap();
        assert_eq!(
            store.meeting_suggester(&all).unwrap().suggest(&all[3]),
            None
        );
    }
}
