//! Toplantı → proje önerileri: projesi belli olmayan toplantı için tek tıkla kabul edilecek
//! bir proje. Hiçbir şey kendiliğinden atanmaz. Tamamen yerel çalışır.
//!
//! İşaretler (her biri projeye puan verir, puanlar toplanır):
//!
//! - **Geçmiş**: daha önce bir projeye düşen seriler (elle atanan ya da kurala uyan). Aynı
//!   konulu seri ([`SAME_SERIES`]); katılımcıların alan adları ([`DOMAIN`]; kullanıcının kendi
//!   alan adı, yani katılımcılı toplantılarda en sık geçen, ve gmail.com gibi genel alan adları
//!   sayılmaz); düzenleyen ([`ORGANIZER`]; kullanıcının kendisi değilse); konudaki sözcükler
//!   ([`TOKEN`]; "toplantı", "weekly" gibi sözcükler sayılmaz). Takvim katılımcı yayımlamıyorsa (Outlook bunları
//!   yalnızca "Tüm ayrıntılar" düzeyinde yazar) konu ve düzenleyen yeter.
//!   Bir özelliğin puanı ağırlığı × saflığı (o projeye giden seri payı) × güvenidir
//!   (`n / (n + 1)`: tek seri 0,5, iki seri 0,67, üç seri 0,75).
//! - **Ad**: konuda projenin adı ([`NAME`]), müşterinin adı ([`CLIENT`]; müşterinin birkaç
//!   etkin projesi varsa son dönemde en çok kullanılanı) ya da bir başlık kuralının bir
//!   projeye bağladığı iş anahtarı öneki (`LOY-12`, [`ISSUE_KEY`]) geçiyor.
//!
//! En yüksek puanlı proje, puanı en az [`THRESHOLD`] ve ikinci projenin en az
//! [`MARGIN`] katıysa önerilir; değilse (belirsiz) öneri yok. Arşivdeki projeler önerilmez;
//! geçmişte onlara giden seriler yine de saflığı düşürür (alan adı iki projeye gittiyse
//! hiçbirine güvenilmez). [`MIN_LENGTH`]'ten kısa toplantılara öneri yapılmaz.

use std::collections::{HashMap, HashSet};

use chrono::Duration;
use serde::Serialize;

use crate::classify::{Rule, RuleField};
use crate::search::fold;
use crate::timesheet::{Meeting, issue_keys};

/// Bundan kısa toplantıya öneri yapılmaz (atama önerileri en az 15 dakika).
pub const MIN_LENGTH: Duration = Duration::minutes(15);
/// Önerilecek en düşük toplam puan.
pub const THRESHOLD: f64 = 1.0;
/// En iyi proje ikinciden en az bu kat yüksek puanlı olmalı.
pub const MARGIN: f64 = 2.0;

/// Aynı konulu seri daha önce bu projeye atandı (Outlook seriyi yeniden gönderince UID değişir).
const SAME_SERIES: f64 = 2.0;
/// Konuda projenin adı geçiyor.
const NAME: f64 = 2.0;
/// Konuda bir kuralın projeye bağladığı iş anahtarı öneki geçiyor.
const ISSUE_KEY: f64 = 2.0;
/// Konuda müşterinin adı geçiyor (projesi seçilerek).
const CLIENT: f64 = 1.5;
/// Katılımcı alan adı (geçmiş).
const DOMAIN: f64 = 1.6;
/// Düzenleyen adres (geçmiş).
const ORGANIZER: f64 = 1.4;
/// Konudaki bir sözcük (geçmiş); en güçlü iki sözcük toplanır. İki seride geçen sözcük
/// (1,5 × 0,67 = 1,0) ya da tek seride geçen iki sözcük (2 × 0,75) tek başına yeter.
const TOKEN: f64 = 1.5;
const MAX_TOKENS: usize = 2;
/// Sözcük en az bu uzunlukta olmalı (karakter).
const MIN_TOKEN: usize = 3;

/// Toplantı konusunda bir şey söylemeyen sözcükler (katlanmış: [`fold`]).
const STOPWORDS: &[&str] = &[
    "toplanti",
    "toplantisi",
    "toplantı",
    "toplantısı",
    "meeting",
    "sync",
    "weekly",
    "daily",
    "haftalik",
    "haftalık",
    "gunluk",
    "günlük",
    "call",
    "gorusme",
    "görüşme",
    "görüşmesi",
    "review",
    "update",
    "status",
    "durum",
    "the",
    "and",
    "for",
    "with",
    "ile",
    "için",
    "icin",
    "ve",
    "hakkında",
    "online",
    "teams",
    "zoom",
    "invitation",
    "davet",
    "updated",
    "güncellendi",
    "fwd",
];

/// Kişisel e-posta sağlayıcıları: kimin firması olduğunu söylemez.
const PUBLIC_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "hotmail.com.tr",
    "live.com",
    "msn.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "yahoo.com",
    "yahoo.com.tr",
    "yandex.com",
    "yandex.com.tr",
    "proton.me",
    "protonmail.com",
    "aol.com",
    "gmx.com",
    "gmx.de",
];

/// Önerilen proje ve kısa gerekçe ("katılımcılar @acme.com").
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSuggestion {
    pub project_id: String,
    pub reason: String,
}

/// Önerilebilecek bir proje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    /// Bağlı olduğu müşterinin adı.
    pub client: Option<String>,
    pub archived: bool,
}

/// Önerilerin girdileri.
#[derive(Debug, Clone, Copy)]
pub struct SuggestInput<'a> {
    pub projects: &'a [ProjectInfo],
    /// Projesi belli seriler (serinin bir örneği) ve projeleri.
    pub history: &'a [(Meeting, String)],
    /// Kullanıcının kendi alan adını bulmak için bakılan tüm toplantılar (geçmiş dahil).
    pub all: &'a [Meeting],
    /// Etkin kurallar (iş anahtarı önekleri başlık kurallarından).
    pub rules: &'a [Rule],
    /// Proje → son dönemdeki kullanım (saat): müşterinin projeleri arasında seçmek için.
    pub usage: &'a HashMap<String, f64>,
}

/// Bir özelliğin (alan adı, düzenleyen, sözcük) gittiği projeler: proje → seri sayısı.
type Votes = HashMap<String, HashMap<String, usize>>;

/// Geçmişten öğrenilmiş öneri modeli; bir kez kurulur, her toplantı için [`Self::suggest`].
#[derive(Debug, Clone, Default)]
pub struct MeetingSuggester {
    /// Etkin proje kimliği → ad.
    active: HashMap<String, String>,
    /// (Katlanmış ad, proje) — uzun ad önce.
    names: Vec<(String, String)>,
    /// (Katlanmış müşteri adı, görünen ad, müşterinin etkin projeleri).
    clients: Vec<(String, String, Vec<String>)>,
    /// İş anahtarı öneki → projeler.
    key_prefixes: HashMap<String, HashSet<String>>,
    usage: HashMap<String, f64>,
    own_domains: HashSet<String>,
    own_address: Option<String>,
    /// Katlanmış konu → projeler.
    subjects: HashMap<String, HashSet<String>>,
    domains: Votes,
    organizers: Votes,
    tokens: Votes,
}

/// `firma.com.tr`, `eu.firma.com` → kayıtlı alan adı (`firma.com.tr`, `firma.com`).
fn domain_of(email: &str) -> Option<String> {
    let host = email.rsplit_once('@')?.1.trim_end_matches('.');
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return None;
    }
    // İkinci düzey ülke alan adları: com.tr, co.uk, org.tr…
    let sld = [
        "com", "co", "org", "net", "gov", "edu", "ac", "gen", "web", "bel",
    ];
    let take = if labels.len() >= 3
        && labels[labels.len() - 1].len() == 2
        && sld.contains(&labels[labels.len() - 2])
    {
        3
    } else {
        2
    };
    Some(labels[labels.len() - take..].join("."))
}

/// Konunun karşılaştırma biçimi: katlanmış, tek boşluklu.
fn normalize_subject(subject: &str) -> String {
    fold(subject)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Konudaki anlamlı sözcükler (katlanmış, benzersiz, sırayla).
fn tokens(subject: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in fold(subject).split(|c: char| !c.is_alphanumeric()) {
        let ok = word.chars().count() >= MIN_TOKEN
            && !word.chars().all(|c| c.is_ascii_digit())
            && !STOPWORDS.iter().any(|s| fold(s) == word);
        if ok && !out.iter().any(|w| w == word) {
            out.push(word.to_string());
        }
    }
    out
}

/// Sözcüğün konudaki yazılışı (gerekçede gösterilir).
fn display_word(subject: &str, token: &str) -> String {
    subject
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| fold(w) == token)
        .unwrap_or(token)
        .to_string()
}

/// `needle` (katlanmış) `haystack`'te (katlanmış) sözcük sınırlarında geçiyor mu?
fn contains_words(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Başlık kuralı deseninin iş anahtarı öneki: `LOY-`, `LOY-12` → `LOY`.
fn rule_key_prefix(pattern: &str) -> Option<String> {
    let p = pattern.trim().to_uppercase();
    let (prefix, number) = p.split_once('-')?;
    let ok = (2..=10).contains(&prefix.len())
        && prefix.starts_with(|c: char| c.is_ascii_uppercase())
        && prefix
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && number.chars().all(|c| c.is_ascii_digit());
    ok.then(|| prefix.to_string())
}

/// Dış alan adları: kendi ve genel alan adları atılmış.
fn external_domains(m: &Meeting, own: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for email in m.attendees.iter().chain(&m.organizer) {
        if let Some(d) = domain_of(email)
            && !own.contains(&d)
            && !PUBLIC_DOMAINS.contains(&d.as_str())
            && !out.contains(&d)
        {
            out.push(d);
        }
    }
    out
}

/// Özelliğin `project` için puanı: saflık × güven (`n / (n + 1)`). Arşivdeki projelere giden
/// seriler de `n`'e sayılır.
fn strength(votes: &HashMap<String, usize>, project: &str) -> f64 {
    let n: usize = votes.values().sum();
    let c = votes.get(project).copied().unwrap_or(0);
    if n == 0 || c == 0 {
        return 0.0;
    }
    (c as f64 / n as f64) * (n as f64 / (n as f64 + 1.0))
}

/// Bir projenin toplanan puanı ve en güçlü işaretin gerekçesi.
#[derive(Default)]
struct Score {
    total: f64,
    best: f64,
    reason: String,
}

impl Score {
    fn add(&mut self, points: f64, reason: impl FnOnce() -> String) {
        if points <= 0.0 {
            return;
        }
        self.total += points;
        if points > self.best {
            self.best = points;
            self.reason = reason();
        }
    }
}

impl MeetingSuggester {
    pub fn new(input: &SuggestInput) -> Self {
        let mut s = Self {
            usage: input.usage.clone(),
            ..Self::default()
        };
        for p in input.projects.iter().filter(|p| !p.archived) {
            s.active.insert(p.id.clone(), p.name.clone());
            let name = normalize_subject(&p.name);
            if name.chars().count() >= MIN_TOKEN {
                s.names.push((name, p.id.clone()));
            }
            if let Some(client) = p.client.as_deref() {
                let key = normalize_subject(client);
                if key.chars().count() < MIN_TOKEN {
                    continue;
                }
                match s.clients.iter_mut().find(|(k, _, _)| *k == key) {
                    Some((_, _, projects)) => projects.push(p.id.clone()),
                    None => s
                        .clients
                        .push((key, client.trim().to_string(), vec![p.id.clone()])),
                }
            }
        }
        s.names.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        for rule in input.rules.iter().filter(|r| r.field == RuleField::Title) {
            if let Some(prefix) = rule_key_prefix(&rule.pattern) {
                s.key_prefixes
                    .entry(prefix)
                    .or_default()
                    .insert(rule.tag_id.clone());
            }
        }

        // Kendi adres ve alan adı: katılımcısı yayımlanan toplantılarda en sık geçenler
        // (kullanıcı her toplantının katılımcısıdır). Yalnızca düzenleyeni olan takvimde
        // bilinemez: en sık düzenleyen bir müşteri de olabilir.
        let mut addresses: HashMap<&str, usize> = HashMap::new();
        let mut domains: HashMap<String, usize> = HashMap::new();
        for m in input.all.iter().filter(|m| !m.attendees.is_empty()) {
            let people: HashSet<&str> = m
                .attendees
                .iter()
                .chain(&m.organizer)
                .map(String::as_str)
                .collect();
            let hosts: HashSet<String> = people.iter().filter_map(|e| domain_of(e)).collect();
            for p in people {
                *addresses.entry(p).or_default() += 1;
            }
            for d in hosts {
                *domains.entry(d).or_default() += 1;
            }
        }
        s.own_address = addresses
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .map(|(a, _)| a.to_string());
        if let Some(max) = domains.values().copied().max() {
            s.own_domains = domains
                .into_iter()
                .filter(|(d, n)| *n == max && !PUBLIC_DOMAINS.contains(&d.as_str()))
                .map(|(d, _)| d)
                .collect();
            // Eşitlikte (hep aynı müşteriyle toplantı) en sık geçen adresin alan adı seçilir.
            if let Some(own) = s.own_address.as_deref().and_then(domain_of)
                && s.own_domains.len() > 1
                && s.own_domains.contains(&own)
            {
                s.own_domains = HashSet::from([own]);
            }
        }

        // Geçmiş: her seri bir kez sayılır.
        let mut uids: HashSet<&str> = HashSet::new();
        for (m, project) in input.history {
            if !uids.insert(m.uid.as_str()) {
                continue;
            }
            let subject = normalize_subject(&m.subject);
            if !subject.is_empty() {
                s.subjects
                    .entry(subject)
                    .or_default()
                    .insert(project.clone());
            }
            let vote = |votes: &mut Votes, key: String| {
                *votes
                    .entry(key)
                    .or_default()
                    .entry(project.clone())
                    .or_default() += 1;
            };
            for d in external_domains(m, &s.own_domains) {
                vote(&mut s.domains, d);
            }
            // Kendi alan adından bir iş arkadaşı (örn. projenin yöneticisi) da işarettir;
            // yalnızca kullanıcının kendisi sayılmaz.
            if let Some(o) = &m.organizer
                && s.own_address.as_ref() != Some(o)
            {
                vote(&mut s.organizers, o.clone());
            }
            for t in tokens(&m.subject) {
                vote(&mut s.tokens, t);
            }
        }
        s
    }

    /// Toplantının önerilen projesi; emin değilse `None`.
    pub fn suggest(&self, m: &Meeting) -> Option<MeetingSuggestion> {
        if m.end - m.start < MIN_LENGTH || self.active.is_empty() {
            return None;
        }
        let mut scores: HashMap<&str, Score> = HashMap::new();
        let subject = normalize_subject(&m.subject);

        // Aynı konulu seri: tek projeye gittiyse.
        if let Some(projects) = self.subjects.get(&subject)
            && projects.len() == 1
            && let Some(p) = projects.iter().next()
        {
            scores.entry(p).or_default().add(SAME_SERIES, || {
                format!(
                    "bu seri daha önce {} projesine atandı",
                    self.active.get(p).map_or("", String::as_str)
                )
            });
        }

        // Proje adı: en uzun eşleşen adlar (kısa ad uzun adın parçasıysa sayılmaz).
        let mut matched: Vec<&str> = Vec::new();
        for (name, project) in &self.names {
            if contains_words(&subject, name) && !matched.iter().any(|n| n.contains(name.as_str()))
            {
                matched.push(name);
                let shown = self.active.get(project).cloned().unwrap_or_default();
                scores
                    .entry(project)
                    .or_default()
                    .add(NAME, || format!("konuda '{shown}' geçiyor"));
            }
        }

        // Müşteri adı: müşterinin tek etkin projesi ya da en çok kullanılanı (eşitlikte yok).
        for (key, client, projects) in &self.clients {
            if !contains_words(&subject, key) {
                continue;
            }
            let usage = |p: &String| self.usage.get(p).copied().unwrap_or(0.0);
            let pick = if projects.len() == 1 {
                projects.first()
            } else {
                let mut ranked: Vec<&String> = projects.iter().collect();
                ranked.sort_by(|a, b| usage(b).total_cmp(&usage(a)));
                (usage(ranked[0]) > 0.0 && usage(ranked[0]) > usage(ranked[1])).then(|| ranked[0])
            };
            if let Some(p) = pick {
                scores
                    .entry(p)
                    .or_default()
                    .add(CLIENT, || format!("konuda müşteri '{client}' geçiyor"));
            }
        }

        // İş anahtarı: öneki bir kuralla tek projeye bağlıysa.
        for key in issue_keys(&m.subject) {
            let prefix = key.split('-').next().unwrap_or_default();
            if let Some(projects) = self.key_prefixes.get(prefix)
                && projects.len() == 1
                && let Some(p) = projects.iter().next()
            {
                scores
                    .entry(p)
                    .or_default()
                    .add(ISSUE_KEY, || format!("konuda {key} geçiyor"));
            }
        }

        // Geçmişten öğrenilenler.
        let candidates: HashSet<&String> = self
            .domains
            .values()
            .chain(self.organizers.values())
            .chain(self.tokens.values())
            .flat_map(|v| v.keys())
            .collect();
        let domains = external_domains(m, &self.own_domains);
        let words = tokens(&m.subject);
        for project in candidates {
            let score = scores.entry(project).or_default();
            // Birden çok dış alan adı tek işarettir: en güçlüsü.
            if let Some((d, v)) = domains
                .iter()
                .filter_map(|d| Some((d, strength(self.domains.get(d)?, project))))
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                score.add(DOMAIN * v, || format!("katılımcılar @{d}"));
            }
            if let Some(o) = &m.organizer
                && let Some(votes) = self.organizers.get(o)
            {
                score.add(ORGANIZER * strength(votes, project), || {
                    format!("düzenleyen {o}")
                });
            }
            let mut found: Vec<(&String, f64)> = words
                .iter()
                .filter_map(|w| Some((w, strength(self.tokens.get(w)?, project))))
                .filter(|(_, v)| *v > 0.0)
                .collect();
            found.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (w, v) in found.into_iter().take(MAX_TOKENS) {
                score.add(TOKEN * v, || {
                    format!("konuda '{}' geçiyor", display_word(&m.subject, w))
                });
            }
        }

        // Arşivdeki (ya da silinmiş) projeler önerilmez.
        let mut ranked: Vec<(&str, Score)> = scores
            .into_iter()
            .filter(|(p, s)| s.total > 0.0 && self.active.contains_key(*p))
            .collect();
        ranked.sort_by(|a, b| b.1.total.total_cmp(&a.1.total).then(a.0.cmp(b.0)));
        let mut ranked = ranked.into_iter();
        let (project, best) = ranked.next()?;
        let second = ranked.next().map_or(0.0, |(_, s)| s.total);
        (best.total >= THRESHOLD && best.total >= second * MARGIN).then(|| MeetingSuggestion {
            project_id: project.to_string(),
            reason: best.reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::*;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap() + Duration::minutes(min)
    }

    fn meeting(uid: &str, subject: &str, organizer: Option<&str>, attendees: &[&str]) -> Meeting {
        Meeting {
            uid: uid.into(),
            start: t(0),
            end: t(60),
            subject: subject.into(),
            organizer: organizer.map(str::to_string),
            attendees: attendees.iter().map(|a| a.to_string()).collect(),
            ..Meeting::default()
        }
    }

    fn project(id: &str, name: &str, client: Option<&str>) -> ProjectInfo {
        ProjectInfo {
            id: id.into(),
            name: name.into(),
            client: client.map(str::to_string),
            archived: false,
        }
    }

    fn projects() -> Vec<ProjectInfo> {
        vec![
            project("portal", "Portal", Some("Acme")),
            project("app", "Mobil Uygulama", Some("Acme")),
            project("kum", "Kum", None),
        ]
    }

    /// Geçmiş ve adaylarla öneri modeli; `all` geçmiş + adaylar.
    fn suggester(
        projects: &[ProjectInfo],
        history: &[(Meeting, String)],
        others: &[Meeting],
        rules: &[Rule],
        usage: &HashMap<String, f64>,
    ) -> MeetingSuggester {
        let all: Vec<Meeting> = history
            .iter()
            .map(|(m, _)| m.clone())
            .chain(others.iter().cloned())
            .collect();
        MeetingSuggester::new(&SuggestInput {
            projects,
            history,
            all: &all,
            rules,
            usage,
        })
    }

    fn assigned(m: Meeting, p: &str) -> (Meeting, String) {
        (m, p.to_string())
    }

    #[test]
    fn learns_attendee_domain_ignoring_own_and_public() {
        let me = "me@kum.dev";
        let history = [
            assigned(
                meeting(
                    "a",
                    "Sprint planlama",
                    None,
                    &[me, "ali@acme.com", "x@gmail.com"],
                ),
                "portal",
            ),
            assigned(
                meeting(
                    "b",
                    "Tasarım değerlendirme",
                    None,
                    &[me, "veli@eu.acme.com"],
                ),
                "portal",
            ),
            assigned(meeting("c", "İç sunum", None, &[me, "x@gmail.com"]), "kum"),
        ];
        let next = meeting("n", "Yeni konu", None, &[me, "ayse@acme.com"]);
        let s = suggester(
            &projects(),
            &history,
            std::slice::from_ref(&next),
            &[],
            &HashMap::new(),
        );
        assert!(s.own_domains.contains("kum.dev"));
        // Alt alan adı kayıtlı alan adına indirgenir; iki seri → 1,6 × 0,67 ≈ 1,07.
        assert_eq!(
            s.suggest(&next),
            Some(MeetingSuggestion {
                project_id: "portal".into(),
                reason: "katılımcılar @acme.com".into()
            })
        );
        // Kendi alan adı ve genel alan adı bir şey söylemez.
        assert!(!s.domains.contains_key("kum.dev"));
        assert!(!s.domains.contains_key("gmail.com"));
        let only_public = meeting("p", "Başka", None, &[me, "y@gmail.com"]);
        assert_eq!(s.suggest(&only_public), None);
        // Tek seriden öğrenilen alan adı tek başına yetmez (1,6 × 0,5 = 0,8).
        let once = [assigned(
            meeting("a", "Planlama", None, &[me, "ali@beta.com"]),
            "kum",
        )];
        let next = meeting("n", "Başka", None, &[me, "x@beta.com"]);
        let internal = meeting("z", "İç", None, &[me]);
        let s = suggester(
            &projects(),
            &once,
            &[next.clone(), internal],
            &[],
            &HashMap::new(),
        );
        assert_eq!(s.own_domains, HashSet::from(["kum.dev".to_string()]));
        assert_eq!(s.suggest(&next), None);
    }

    #[test]
    fn subject_tokens_and_organizer_without_attendees() {
        // Katılımcısız takvim: konu ve düzenleyen.
        let history = [
            assigned(meeting("a", "Loyalty sprint review", None, &[]), "portal"),
            assigned(
                meeting("b", "Loyalty haftalık toplantı", None, &[]),
                "portal",
            ),
            assigned(meeting("c", "Haftalık toplantı", None, &[]), "kum"),
        ];
        let next = meeting("n", "LOYALTY demo", None, &[]);
        let s = suggester(&projects(), &history, &[], &[], &HashMap::new());
        let got = s.suggest(&next).unwrap();
        assert_eq!(got.project_id, "portal");
        assert_eq!(got.reason, "konuda 'LOYALTY' geçiyor");
        // "Haftalık toplantı" yalnızca durak sözcükler: öneri yok.
        assert_eq!(
            s.suggest(&meeting("m", "Haftalık toplantı 2", None, &[])),
            None
        );
        assert!(!s.tokens.contains_key("haftalik"));

        let history = [
            assigned(meeting("a", "Planlama", Some("pm@acme.com"), &[]), "app"),
            assigned(meeting("b", "Demo", Some("pm@acme.com"), &[]), "app"),
            assigned(meeting("c", "Retro", Some("pm@acme.com"), &[]), "app"),
        ];
        let s = suggester(&projects(), &history, &[], &[], &HashMap::new());
        let got = s
            .suggest(&meeting("n", "Kickoff", Some("pm@acme.com"), &[]))
            .unwrap();
        assert_eq!(got.project_id, "app");
        // Düzenleyenin alan adı da tek başına işarettir; en güçlüsü gerekçe olur.
        assert!(got.reason.contains("acme.com"), "{}", got.reason);
    }

    #[test]
    fn same_subject_series_and_name_matches() {
        let history = [assigned(
            meeting("old", "Togg haftalık durum", None, &[]),
            "kum",
        )];
        let s = suggester(&projects(), &history, &[], &[], &HashMap::new());
        // Outlook seriyi yeniden gönderince UID değişir; konu aynı.
        let got = s
            .suggest(&meeting("new", "Togg  Haftalık durum", None, &[]))
            .unwrap();
        assert_eq!(
            got,
            MeetingSuggestion {
                project_id: "kum".into(),
                reason: "bu seri daha önce Kum projesine atandı".into()
            }
        );
        // Proje adı sözcük olarak geçmeli ("Portalı" değil, "Portal"), Türkçe I katlanır.
        let got = s
            .suggest(&meeting("n", "PORTAL tasarım", None, &[]))
            .unwrap();
        assert_eq!(got.project_id, "portal");
        assert_eq!(got.reason, "konuda 'Portal' geçiyor");
        assert_eq!(
            s.suggest(&meeting("n", "Portalı konuşalım", None, &[])),
            None
        );
        let got = s
            .suggest(&meeting("n", "MOBİL UYGULAMA demo", None, &[]))
            .unwrap();
        assert_eq!(got.project_id, "app");
        // 15 dakikadan kısa toplantıya öneri yok.
        let mut short = meeting("n", "Portal", None, &[]);
        short.end = t(10);
        assert_eq!(s.suggest(&short), None);
    }

    #[test]
    fn client_name_picks_most_used_project() {
        let projects = [
            project("portal", "Portal", Some("Acme")),
            project("app", "Mobil Uygulama", Some("Acme")),
            project("beta", "Kampanya", Some("Beta Holding")),
        ];
        let next = meeting("n", "Acme yönetim kurulu", None, &[]);
        // Kullanım yok: iki projeden biri seçilemez.
        let s = suggester(&projects, &[], &[], &[], &HashMap::new());
        assert_eq!(s.suggest(&next), None);
        let usage = HashMap::from([("app".to_string(), 12.0), ("portal".to_string(), 3.0)]);
        let s = suggester(&projects, &[], &[], &[], &usage);
        assert_eq!(
            s.suggest(&next),
            Some(MeetingSuggestion {
                project_id: "app".into(),
                reason: "konuda müşteri 'Acme' geçiyor".into()
            })
        );
        // Tek projeli müşteri.
        let got = s
            .suggest(&meeting("n", "beta holding ile tanışma", None, &[]))
            .unwrap();
        assert_eq!(got.project_id, "beta");
    }

    #[test]
    fn issue_key_prefix_from_title_rule() {
        let rule = |tag: &str, pattern: &str| Rule {
            id: pattern.into(),
            tag_id: tag.into(),
            field: RuleField::Title,
            pattern: pattern.into(),
        };
        let rules = [
            rule("portal", "LOY-"),
            rule("kum", "kum-12"),
            rule("app", "mobil"),
        ];
        let s = suggester(&projects(), &[], &[], &rules, &HashMap::new());
        let got = s
            .suggest(&meeting("n", "LOY-214 hata analizi", None, &[]))
            .unwrap();
        assert_eq!(got.project_id, "portal");
        assert_eq!(got.reason, "konuda LOY-214 geçiyor");
        assert_eq!(
            s.suggest(&meeting("n", "KUM-3 planlama", None, &[]))
                .map(|g| g.project_id),
            Some("kum".into())
        );
    }

    #[test]
    fn archived_projects_are_never_suggested() {
        let me = "me@kum.dev";
        let mut projects = projects();
        projects[0].archived = true; // Portal
        let history = [
            assigned(
                meeting("a", "Loyalty", None, &[me, "ali@acme.com"]),
                "portal",
            ),
            assigned(
                meeting("b", "Loyalty 2", None, &[me, "ali@acme.com"]),
                "portal",
            ),
            assigned(meeting("c", "İç", None, &[me]), "kum"),
        ];
        let s = suggester(&projects, &history, &[], &[], &HashMap::new());
        assert!(s.domains.contains_key("acme.com"));
        let next = meeting("n", "Portal Loyalty", None, &[me, "v@acme.com"]);
        assert_eq!(s.suggest(&next), None);
        assert_eq!(s.suggest(&meeting("n", "Loyalty", None, &[])), None);
    }

    #[test]
    fn ambiguous_history_gives_no_suggestion() {
        let me = "me@kum.dev";
        let history = [
            assigned(
                meeting("a", "Planlama", None, &[me, "ali@acme.com"]),
                "portal",
            ),
            assigned(meeting("b", "Retro", None, &[me, "ali@acme.com"]), "portal"),
            assigned(meeting("c", "Demo", None, &[me, "ali@acme.com"]), "app"),
            assigned(meeting("d", "Kickoff", None, &[me, "ali@acme.com"]), "app"),
            assigned(meeting("e", "İç", None, &[me]), "kum"),
        ];
        let s = suggester(&projects(), &history, &[], &[], &HashMap::new());
        // Alan adı iki projeye eşit gitti.
        let next = meeting("n", "Yeni", None, &[me, "v@acme.com"]);
        assert_eq!(s.suggest(&next), None);
        // Konuda iki proje adı: belirsiz.
        assert_eq!(s.suggest(&meeting("n", "Portal ve Kum", None, &[])), None);
        // Hiçbir işaret yok.
        assert_eq!(s.suggest(&meeting("n", "Öğle yemeği", None, &[])), None);
    }

    #[test]
    fn small_helpers() {
        assert_eq!(domain_of("a@eu.acme.com").as_deref(), Some("acme.com"));
        assert_eq!(domain_of("a@x.acme.com.tr").as_deref(), Some("acme.com.tr"));
        assert_eq!(domain_of("nope"), None);
        assert_eq!(rule_key_prefix("loy-"), Some("LOY".into()));
        assert_eq!(rule_key_prefix("LOY-12"), Some("LOY".into()));
        assert_eq!(rule_key_prefix("togg"), None);
        assert_eq!(rule_key_prefix("e-posta"), None);
        assert_eq!(
            tokens("Haftalık Toplantı: LOYALTY / Kampanya 2026"),
            ["loyalty", "kampanya"]
        );
    }
}
