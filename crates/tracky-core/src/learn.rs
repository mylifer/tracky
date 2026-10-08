//! Elle atamalardan kural önerileri: kullanıcının sık sık aynı türden süreyi aynı projeye
//! elle atadığı görülünce bunu bir kurala çevirmesi önerilir; böylece elle iş azalır.
//!
//! Elle atanan (`project_id` verilmiş, "Projesiz" olmayan) ve mevcut kurallarla zaten o
//! projeye düşmeyen oturumlardan aday desenler çıkar:
//! - başlıktaki iş anahtarının öneki (`LOY-214`, `LOY-87` → başlık kuralı `LOY-`),
//! - başlıktan tanınan proje adı (editör klasörü, GitHub reposu; [`crate::suggest`]),
//! - tarayıcı adresinin alanı ya da ilk bir iki yol parçası (`github.com/firma/repo`).
//!
//! Aday ancak tekrar eden bir alışkanlıksa önerilir: en az iki ayrı günde ya da üç ayrı
//! atamada, toplam en az 15 dakika. Sonra kural önizlemesiyle ([`preview_rule`]) dönemin
//! tamamına bakılır: uyan ve bir projeye düşen sürenin en az %80'i o projede olmalı, kural
//! başka projelerden kayda değer süre almamalı. Yoksayılan ve zaten bir kuralla kapsanan
//! desen önerilmez; aynı elle atamaları kapsayan iki adaydan yalnızca öndeki kalır.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::classify::{Classifier, NO_PROJECT, Rule, RuleField, Tag, TagKind};
use crate::inbox::{MIN_GROUP_SECS, RulePreview, preview_rule};
use crate::model::Session;

/// Öneriler bu kadar günlük geçmişten öğrenilir.
pub const LEARN_DAYS: i64 = 30;
/// Bundan kısa elle atanmış süre kural önerisi olmaz (atama önerileriyle aynı alt sınır).
pub const MIN_RULE_SECS: i64 = MIN_GROUP_SECS;
/// Alışkanlık sayılmak için en az bu kadar ayrı gün…
pub const MIN_DAYS: usize = 2;
/// …ya da bu kadar ayrı atama.
pub const MIN_ASSIGNMENTS: usize = 3;
/// Uyan ve bir projeye düşen sürenin en az bu yüzdesi önerilen projede olmalı.
const MIN_SHARE_PCT: i64 = 80;
/// Kural, uyan sürenin en çok bu yüzdesini başka projelerden alabilir.
const MAX_TAKEN_PCT: i64 = 10;
/// Aynı desendeki elle atanmış oturumlar arasında bundan uzun ara varsa ayrı atama sayılır.
const ASSIGNMENT_GAP: Duration = Duration::minutes(30);
/// Önizlemesi hesaplanan en çok aday (önizleme bütün dönemi dolaşır).
const MAX_PREVIEWED: usize = 40;
/// İkinci aday elle atamalarının bu yüzdesi öndeki adayca kapsanıyorsa önerilmez.
const COVERED_PCT: i64 = 80;
/// Adres yolunun en çok bu kadar parçası desene girer (`github.com/firma/repo`).
const MAX_PATH_SEGMENTS: usize = 2;

/// Adayın nereden çıktığı (arayüzdeki cümle buna göre kurulur).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CandidateSource {
    /// Başlıktaki iş anahtarının öneki (`LOY-`).
    IssueKey,
    /// Başlıktan tanınan proje adı (editör klasörü, GitHub reposu…).
    TitleWord,
    /// Tarayıcı adresi (alan ya da alan/yol).
    Site,
}

/// Önerilen kural ve gerekçesi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleCandidate {
    /// Yoksayma anahtarı: `rule:<proje>:<alan>:<desen>`.
    pub key: String,
    pub project_id: String,
    pub project_name: String,
    pub field: RuleField,
    pub pattern: String,
    pub source: CandidateSource,
    /// Ayrı elle atama sayısı (aralarında yarım saatten uzun ara olan parçalar).
    pub assignments: usize,
    /// Elle atamanın yapıldığı ayrı gün sayısı.
    pub days: usize,
    /// Desene uyan, elle bu projeye atanmış süre.
    pub manual_seconds: i64,
    /// Kural eklenseydi dönemde ne değişirdi.
    pub preview: RulePreview,
}

impl RuleCandidate {
    /// Eklenecek kural (kimliği boş; depo ekler).
    pub fn rule(&self) -> Rule {
        Rule {
            id: String::new(),
            tag_id: self.project_id.clone(),
            field: self.field,
            pattern: self.pattern.clone(),
        }
    }

    /// Sıralama ölçüsü: kural olsaydı kendiliğinden projeye düşecek süre (şimdi atanmamış
    /// olan ve elle atanmak zorunda kalınan).
    fn score(&self) -> i64 {
        self.preview.gained_seconds + self.manual_seconds
    }
}

/// Yoksayma anahtarı; desen kuralın karşılaştırma biçimindedir (büyük/küçük harf farkı yok).
pub fn candidate_key(project_id: &str, rule: &Rule) -> String {
    format!(
        "rule:{project_id}:{}:{}",
        rule.field.as_str(),
        rule.prepared_pattern()
    )
}

/// Öğrenmenin girdileri (oturumlar dışında).
pub struct LearnInput<'a> {
    pub tags: &'a [Tag],
    /// Etkin kurallar.
    pub rules: &'a [Rule],
    /// Arşivdeki projeler: önerilmez.
    pub archived: &'a HashSet<String>,
    /// Yoksayılan öneri anahtarları ([`candidate_key`]).
    pub dismissed: &'a HashSet<String>,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Anın yerel günü (ayrı gün sayısı için).
    pub day_of: &'a dyn Fn(DateTime<Utc>) -> NaiveDate,
    /// Verilirse yalnızca bu projenin adayları (atamadan hemen sonra; daha hızlı).
    pub project: Option<&'a str>,
}

/// Oturumun aday desenleri: (alan, desen, kaynak).
fn patterns_of(s: &Session) -> Vec<(RuleField, String, CandidateSource)> {
    let mut out = Vec::new();
    for key in crate::timesheet::issue_keys(&s.title) {
        if let Some((prefix, _)) = key.split_once('-') {
            let p = format!("{prefix}-");
            if !out.iter().any(|(_, q, _)| *q == p) {
                out.push((RuleField::Title, p, CandidateSource::IssueKey));
            }
        }
    }
    if let Some(name) = crate::suggest::project_from_title(&s.app_id, &s.title, s.domain.as_deref())
    {
        out.push((RuleField::Title, name, CandidateSource::TitleWord));
    }
    if crate::browser::is_browser(&s.app_id)
        && let Some(address) = s.url.as_deref().and_then(crate::url_util::host_path)
    {
        let mut parts = address.split('/');
        let host = parts.next().unwrap_or_default().to_string();
        if !host.is_empty() {
            let mut pattern = host;
            out.push((RuleField::Domain, pattern.clone(), CandidateSource::Site));
            for seg in parts.take(MAX_PATH_SEGMENTS) {
                if !useful_segment(seg) {
                    break;
                }
                pattern = format!("{pattern}/{seg}");
                out.push((RuleField::Domain, pattern.clone(), CandidateSource::Site));
            }
        }
    }
    out
}

/// Kurala girmeye değer yol parçası: kimlik, sayı ya da iş anahtarı (`loy-214`) değil.
fn useful_segment(seg: &str) -> bool {
    let digits = seg.chars().filter(char::is_ascii_digit).count();
    let issue = seg.rsplit_once('-').is_some_and(|(p, n)| {
        !p.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
    });
    (1..=40).contains(&seg.chars().count())
        && seg.chars().any(char::is_alphabetic)
        && digits * 3 < seg.chars().count()
        && !issue
}

/// Elle atamalardan kural adayları, en çok kazandırandan aza. Her biri kalite eşiklerini
/// geçmiştir (bkz. modül açıklaması); arayüz ilk birkaçını gösterir.
pub fn rule_candidates(sessions: &[Session], input: &LearnInput) -> Vec<RuleCandidate> {
    let LearnInput {
        tags,
        rules,
        archived,
        dismissed,
        from,
        to,
        day_of,
        project,
    } = *input;
    let projects: HashMap<&str, &str> = tags
        .iter()
        .filter(|t| t.kind == TagKind::Project && !archived.contains(&t.id))
        .map(|t| (t.id.as_str(), t.name.as_str()))
        .collect();
    let classifier = Classifier::new(tags, rules);
    let existing: HashSet<String> = rules.iter().map(|r| candidate_key(&r.tag_id, r)).collect();

    struct Acc {
        field: RuleField,
        pattern: String,
        source: CandidateSource,
        ms: i64,
        days: HashSet<NaiveDate>,
        spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
        /// Desene uyan elle atanmış oturumlar (dizin, süre ms).
        members: Vec<(usize, i64)>,
    }
    // (proje, alan, hazırlanmış desen) → birikim.
    let mut accs: HashMap<(String, &'static str, String), Acc> = HashMap::new();
    for (i, s) in sessions.iter().enumerate() {
        let Some(p) = s.project_id.as_deref() else {
            continue;
        };
        let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
        if ms <= 0
            || p == NO_PROJECT
            || !projects.contains_key(p)
            || project.is_some_and(|only| only != p)
            || s.is_idle()
            || s.is_manual()
        {
            continue;
        }
        // Kurallar zaten bu projeye yazıyorsa elle atama gerekmemiştir; öğrenilecek bir şey yok.
        let by_rules = Session {
            project_id: None,
            block_from: None,
            ..s.clone()
        };
        if classifier.classify(&by_rules).project.as_deref() == Some(p) {
            continue;
        }
        for (field, pattern, source) in patterns_of(s) {
            let rule = Rule {
                id: String::new(),
                tag_id: p.to_string(),
                field,
                pattern,
            };
            let acc = accs
                .entry((p.to_string(), field.as_str(), rule.prepared_pattern()))
                .or_insert_with(|| Acc {
                    field,
                    pattern: rule.pattern.clone(),
                    source,
                    ms: 0,
                    days: HashSet::new(),
                    spans: Vec::new(),
                    members: Vec::new(),
                });
            acc.ms += ms;
            acc.days.insert(day_of(s.started_at.max(from)));
            acc.spans.push((s.started_at, s.ended_at));
            acc.members.push((i, ms));
        }
    }

    // Önce elle atama eşikleri (ucuz), sonra en uzun adayların önizlemesi (pahalı).
    let mut habits: Vec<((String, &'static str, String), Acc, usize)> = accs
        .into_iter()
        .filter_map(|(key, mut acc)| {
            acc.spans.sort();
            let mut assignments = 0;
            let mut last_end: Option<DateTime<Utc>> = None;
            for (start, end) in &acc.spans {
                if last_end.is_none_or(|e| *start - e > ASSIGNMENT_GAP) {
                    assignments += 1;
                }
                last_end = Some(last_end.map_or(*end, |e| e.max(*end)));
            }
            let habit = acc.days.len() >= MIN_DAYS || assignments >= MIN_ASSIGNMENTS;
            (habit && acc.ms / 1000 >= MIN_RULE_SECS).then_some((key, acc, assignments))
        })
        .collect();
    habits.sort_by(|a, b| b.1.ms.cmp(&a.1.ms).then(a.0.cmp(&b.0)));

    let mut out: Vec<(RuleCandidate, Vec<(usize, i64)>)> = Vec::new();
    for ((project_id, ..), acc, assignments) in habits {
        let field = acc.field;
        if out.len() >= MAX_PREVIEWED {
            break;
        }
        let rule = Rule {
            id: String::new(),
            tag_id: project_id.clone(),
            field,
            pattern: acc.pattern.clone(),
        };
        let key = candidate_key(&project_id, &rule);
        if dismissed.contains(&key) || existing.contains(&key) {
            continue;
        }
        let preview = preview_rule(sessions, tags, rules, &rule, from, to);
        // Uyan ve bir projede (ya da elle "Projesiz") olan süre: önerilen projede mi?
        let known = preview.already_seconds + preview.taken_seconds + preview.blocked_seconds;
        let pure = preview.already_seconds * 100 >= MIN_SHARE_PCT * known;
        let gentle = preview.taken_seconds * 100 <= MAX_TAKEN_PCT * preview.matched_seconds;
        // Kural, öğrenildiği oturumları elle atama olmadan da projeye yazmalı: aynı desende
        // önce gelen bir kural (başka projenin) varsa hiçbir şey değiştirmezdi.
        let mut with = rules.to_vec();
        with.push(rule.clone());
        let after = Classifier::new(tags, &with);
        let effective: i64 = acc
            .members
            .iter()
            .filter(|(i, _)| {
                let by_rules = Session {
                    project_id: None,
                    block_from: None,
                    ..sessions[*i].clone()
                };
                after.classify(&by_rules).project.as_deref() == Some(project_id.as_str())
            })
            .map(|(_, ms)| ms)
            .sum();
        if !(pure && gentle && effective * 100 >= MIN_SHARE_PCT * acc.ms) {
            continue;
        }
        out.push((
            RuleCandidate {
                key,
                project_name: projects
                    .get(project_id.as_str())
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                project_id,
                field,
                pattern: acc.pattern,
                source: acc.source,
                assignments,
                days: acc.days.len(),
                manual_seconds: acc.ms / 1000,
                preview,
            },
            acc.members,
        ));
    }

    // Sıralama: kazandıran önce; eşitse önce başlık sonra site, daha özel (uzun) desen önce:
    // geniş desen (`github.com`) ancak daha çok kazandırıyorsa öne geçer.
    let field_rank = |f: RuleField| match f {
        RuleField::Title => 0,
        RuleField::Domain => 1,
        RuleField::App => 2,
    };
    out.sort_by(|(a, _), (b, _)| {
        b.score()
            .cmp(&a.score())
            .then(field_rank(a.field).cmp(&field_rank(b.field)))
            .then(b.pattern.len().cmp(&a.pattern.len()))
            .then(a.key.cmp(&b.key))
    });
    // Aynı elle atamaları kapsayan ikinci aday (örn. `github.com/firma` varken
    // `github.com/firma/repo`) önerilmez: bir kural yeter.
    let mut kept: Vec<(RuleCandidate, HashSet<usize>)> = Vec::new();
    for (c, members) in out {
        let total: i64 = members.iter().map(|(_, ms)| ms).sum();
        let covered = kept.iter().any(|(k, set)| {
            k.project_id == c.project_id
                && members
                    .iter()
                    .filter(|(i, _)| set.contains(i))
                    .map(|(_, ms)| ms)
                    .sum::<i64>()
                    * 100
                    >= COVERED_PCT * total
        });
        if !covered {
            let set = members.iter().map(|(i, _)| *i).collect();
            kept.push((c, set));
        }
    }
    kept.into_iter().map(|(c, _)| c).collect()
}

/// Az önce elle atanan oturumlardan (`assigned`) en az birine uyan, o projenin en iyi adayı.
pub fn candidate_for(
    candidates: Vec<RuleCandidate>,
    project_id: &str,
    assigned: &[Session],
) -> Option<RuleCandidate> {
    candidates.into_iter().find(|c| {
        let rule = c.rule();
        let pattern = rule.prepared_pattern();
        c.project_id == project_id && assigned.iter().any(|s| rule.matches_session(&pattern, s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn day(d: i64, min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, 9, 0, 0).unwrap()
            + Duration::days(d)
            + Duration::minutes(min)
    }

    /// `d`. gün, `from`–`to` dakikaları arasında oturum.
    fn s(app: &str, title: &str, url: Option<&str>, d: i64, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.rsplit('.').next().unwrap_or(app).into(),
            title: title.into(),
            url: url.map(Into::into),
            domain: url.and_then(crate::url_util::domain_of),
            started_at: day(d, from),
            ended_at: day(d, to),
            category_id: None,
            project_id: None,
            block_from: None,
        }
    }

    fn manual(mut x: Session, project: &str) -> Session {
        x.project_id = Some(project.into());
        x
    }

    fn project(id: &str) -> Tag {
        Tag {
            id: id.into(),
            kind: TagKind::Project,
            name: id.to_uppercase(),
            color: 1,
        }
    }

    fn rule(tag: &str, field: RuleField, pattern: &str) -> Rule {
        Rule {
            id: Uuid::new_v4().to_string(),
            tag_id: tag.into(),
            field,
            pattern: pattern.into(),
        }
    }

    struct Fixture {
        tags: Vec<Tag>,
        rules: Vec<Rule>,
        archived: HashSet<String>,
        dismissed: HashSet<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                tags: vec![project("loy"), project("kum"), project("togg")],
                rules: Vec::new(),
                archived: HashSet::new(),
                dismissed: HashSet::new(),
            }
        }

        fn run(&self, sessions: &[Session], only: Option<&str>) -> Vec<RuleCandidate> {
            let day_of = |t: DateTime<Utc>| t.date_naive();
            rule_candidates(
                sessions,
                &LearnInput {
                    tags: &self.tags,
                    rules: &self.rules,
                    archived: &self.archived,
                    dismissed: &self.dismissed,
                    from: day(-1, 0),
                    to: day(30, 0),
                    day_of: &day_of,
                    project: only,
                },
            )
        }
    }

    const JIRA: &str = "com.google.Chrome";

    /// İki günde elle Loyalty'ye atanmış LOY- işleri ve atanmamış bir LOY- işi.
    fn loyalty() -> Vec<Session> {
        vec![
            manual(
                s("com.apple.mail", "LOY-214 Kampanya", None, 0, 0, 20),
                "loy",
            ),
            manual(
                s("com.apple.mail", "Re: LOY-87 puanlar", None, 1, 0, 15),
                "loy",
            ),
            s("com.apple.mail", "LOY-90 sepet", None, 2, 0, 25),
        ]
    }

    #[test]
    fn learns_the_issue_key_prefix_from_manual_assignments() {
        let f = Fixture::new();
        let out = f.run(&loyalty(), None);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!((c.field, c.pattern.as_str()), (RuleField::Title, "LOY-"));
        assert_eq!(c.source, CandidateSource::IssueKey);
        assert_eq!(
            (c.project_id.as_str(), c.project_name.as_str()),
            ("loy", "LOY")
        );
        assert_eq!((c.days, c.assignments, c.manual_seconds), (2, 2, 35 * 60));
        // Atanmamış LOY-90 kazanılır; elle atananlar zaten projede.
        assert_eq!(c.preview.gained_seconds, 25 * 60);
        assert_eq!(c.preview.already_seconds, 35 * 60);
        assert_eq!(c.key, "rule:loy:title:loy-");
    }

    #[test]
    fn needs_a_habit_and_at_least_fifteen_minutes() {
        let f = Fixture::new();
        // Tek günde tek atama: alışkanlık değil.
        let once = vec![manual(
            s("com.apple.mail", "LOY-214", None, 0, 0, 60),
            "loy",
        )];
        assert!(f.run(&once, None).is_empty());
        // Aynı günde, aralarında yarım saatten uzun ara olan üç atama: yeter.
        let thrice: Vec<Session> = [0, 60, 120]
            .iter()
            .map(|m| manual(s("com.apple.mail", "LOY-214", None, 0, *m, m + 10), "loy"))
            .collect();
        let out = f.run(&thrice, None);
        assert_eq!((out.len(), out[0].assignments, out[0].days), (1, 3, 1));
        // Art arda üç oturum tek atamadır.
        let streak: Vec<Session> = [0, 10, 20]
            .iter()
            .map(|m| manual(s("com.apple.mail", "LOY-214", None, 0, *m, m + 10), "loy"))
            .collect();
        assert!(f.run(&streak, None).is_empty());
        // İki gün ama toplam 14 dakika: 15 dakikanın altı önerilmez.
        let short = vec![
            manual(s("com.apple.mail", "LOY-1", None, 0, 0, 7), "loy"),
            manual(s("com.apple.mail", "LOY-2", None, 1, 0, 7), "loy"),
        ];
        assert!(f.run(&short, None).is_empty());
    }

    #[test]
    fn mixed_patterns_and_stealing_rules_are_not_proposed() {
        let f = Fixture::new();
        // LOY- süresinin yarısı elle başka projeye verilmiş: %80 eşiğinin altı.
        let mut mixed = loyalty();
        mixed.push(manual(
            s("com.apple.mail", "LOY-5 togg ortak", None, 3, 0, 40),
            "togg",
        ));
        assert!(f.run(&mixed, None).is_empty());

        // Uyan sürenin %20'si bir kuralla başka projede: site kuralı (başlık kuralından önce
        // gelir) onu alırdı, önerilmez. Aynı başlık kuralı ise hiçbir şey değiştirmezdi: önce
        // eklenen "panel" kuralı hep kazanır.
        let mut f = Fixture::new();
        f.rules.push(rule("togg", RuleField::Title, "panel"));
        let url = "https://github.com/firma/panel";
        let mut sessions = vec![
            manual(s(JIRA, "Ödeme · firma/panel", Some(url), 0, 0, 20), "kum"),
            manual(s(JIRA, "Sepet · firma/panel", Some(url), 1, 0, 20), "kum"),
        ];
        sessions.push(s(JIRA, "panel tasarımı", Some(url), 2, 0, 10));
        assert!(f.run(&sessions, None).is_empty());
        // Başka projeden alınan süre azsa (≤ %10) site kuralı önerilir.
        sessions[2].ended_at = day(2, 4);
        let out = f.run(&sessions, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pattern, "github.com/firma/panel");
        assert_eq!(out[0].preview.taken_seconds, 4 * 60);
    }

    #[test]
    fn dismissed_existing_archived_and_rule_covered_patterns_are_skipped() {
        let mut f = Fixture::new();
        f.dismissed.insert("rule:loy:title:loy-".into());
        assert!(f.run(&loyalty(), None).is_empty());

        // Aynı kural zaten var (büyük/küçük harf farkıyla).
        let mut f = Fixture::new();
        f.rules.push(rule("loy", RuleField::Title, "loy-"));
        assert!(f.run(&loyalty(), None).is_empty());

        // Başka bir kural zaten bu projeye yazıyor: elle atama gerekmemiş, öğrenilmez.
        let mut f = Fixture::new();
        f.rules.push(rule("loy", RuleField::Title, "kampanya"));
        let sessions = vec![
            manual(
                s("com.apple.mail", "LOY-214 Kampanya", None, 0, 0, 20),
                "loy",
            ),
            manual(
                s("com.apple.mail", "LOY-87 kampanya", None, 1, 0, 20),
                "loy",
            ),
        ];
        assert!(f.run(&sessions, None).is_empty());

        // Arşivdeki proje önerilmez; "Projesiz" ve boşta süre öğrenilmez.
        let mut f = Fixture::new();
        f.archived.insert("loy".into());
        assert!(f.run(&loyalty(), None).is_empty());
        let f = Fixture::new();
        let none: Vec<Session> = loyalty()
            .into_iter()
            .map(|x| manual(x, NO_PROJECT))
            .collect();
        assert!(f.run(&none, None).is_empty());
    }

    #[test]
    fn learns_sites_and_keeps_one_rule_for_the_same_assignments() {
        let f = Fixture::new();
        let a = "https://github.com/firma/kum/pull/12";
        let b = "https://github.com/firma/kum/issues";
        let other = "https://github.com/firma/togg";
        let sessions = vec![
            manual(s(JIRA, "Düzeltme", Some(a), 0, 0, 30), "kum"),
            manual(s(JIRA, "Hatalar", Some(b), 1, 0, 30), "kum"),
            manual(s(JIRA, "Togg işleri", Some(other), 2, 0, 30), "togg"),
            manual(s(JIRA, "Togg yine", Some(other), 3, 0, 30), "togg"),
        ];
        let out = f.run(&sessions, None);
        // github.com ve github.com/firma iki projeye bölünür; repo yolları önerilir.
        let mut patterns: Vec<(&str, &str)> = out
            .iter()
            .map(|c| (c.project_id.as_str(), c.pattern.as_str()))
            .collect();
        patterns.sort();
        assert_eq!(
            patterns,
            [
                ("kum", "github.com/firma/kum"),
                ("togg", "github.com/firma/togg")
            ]
        );
        assert!(out.iter().all(|c| c.source == CandidateSource::Site));

        // Yalnızca bir projenin adayları istenebilir.
        let only = f.run(&sessions, Some("togg"));
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].project_id, "togg");
    }

    #[test]
    fn learns_editor_folders_and_ranks_by_time_saved() {
        let f = Fixture::new();
        let code = "com.microsoft.VSCode";
        let sessions = vec![
            manual(s(code, "main.rs — kum", None, 0, 0, 60), "kum"),
            manual(s(code, "lib.rs — kum", None, 1, 0, 60), "kum"),
            s(code, "app.rs — kum", None, 2, 0, 60),
            manual(s("com.apple.mail", "LOY-1", None, 0, 100, 120), "loy"),
            manual(s("com.apple.mail", "LOY-2", None, 1, 100, 120), "loy"),
        ];
        let out = f.run(&sessions, None);
        assert_eq!(
            out.iter().map(|c| c.pattern.as_str()).collect::<Vec<_>>(),
            ["kum", "LOY-"]
        );
        assert_eq!(out[0].source, CandidateSource::TitleWord);
    }

    #[test]
    fn picks_the_candidate_that_covers_the_just_assigned_sessions() {
        let f = Fixture::new();
        let sessions = loyalty();
        let assigned = vec![sessions[1].clone()];
        let c = candidate_for(f.run(&sessions, Some("loy")), "loy", &assigned).unwrap();
        assert_eq!(c.pattern, "LOY-");
        assert!(candidate_for(f.run(&sessions, None), "kum", &assigned).is_none());
        let unrelated = vec![s("com.apple.mail", "Gelen", None, 0, 0, 10)];
        assert!(candidate_for(f.run(&sessions, None), "loy", &unrelated).is_none());
    }

    #[test]
    fn address_segments_that_are_ids_are_not_rule_material() {
        assert!(useful_segment("firma"));
        assert!(useful_segment("browse"));
        assert!(!useful_segment("loy-214"));
        assert!(!useful_segment("123456"));
        assert!(!useful_segment("a1b2c3d4e5"));
        assert!(!useful_segment(""));
    }
}
