//! Atanmamış süre: hiçbir projeye düşmeyen takip edilen süre, gözden geçirmek için gruplanır.
//!
//! Tarayıcıda adresi bilinen süre siteye, gerisi uygulamaya göre gruplanır; grubun içinde
//! pencere başlıkları ayrı satırdır. Kullanıcı bir grubu ya da başlığı projeye atar (yalnızca o
//! oturumlar) ya da kural ekler (geçmiş ve gelecek bütün uyan süre). Elle "Projesiz" denen
//! oturumlar ve yoksayılan gruplar listeye girmez: onlar zaten gözden geçirilmiştir.
//!
//! Ayrıca bir kuralın etkisi önceden hesaplanır ([`preview_rule`]): kural eklenince ne kadar
//! sürenin projeye geçeceği, bunun ne kadarının başka bir projeden alınacağı.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::classify::{Classifier, NO_PROJECT, Rule, Tag, TagKind};
use crate::model::Session;

/// Bundan kısa grup ve başlık atanmak üzere önerilmez: zaman çizelgesi en az çeyrek saatlik
/// işle ilgilenir, birkaç dakikalık geçişler (bildirime bakmak gibi) gürültüdür.
pub const MIN_GROUP_SECS: i64 = 15 * 60;
/// Grupta ayrı satır olarak gösterilen başlığın en kısa süresi; kısalar `more` olarak sayılır
/// (grubu atamak onları da kapsar).
pub const MIN_ITEM_SECS: i64 = 15 * 60;
/// Bundan kısa boşta süre önerilmez.
pub const MIN_IDLE_SECS: i64 = 15 * 60;
/// Grupta gösterilen en çok başlık; gerisi `more` olarak sayılır.
pub const MAX_ITEMS: usize = 12;
/// Komşu iş: atanmamış oturuma bu kadar yakın atanmış oturumun projesi öneri sayılır.
const NEIGHBOR_GAP: Duration = Duration::minutes(10);

/// Grubun türü: kural da buna göre önerilir (site → web sitesi kuralı, uygulama → uygulama).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    Site,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedItem {
    /// Temizlenmiş pencere başlığı (tarayıcı adı atılmış); boş olabilir.
    pub title: String,
    pub seconds: i64,
    /// Başlıktan tanınan proje adı (editör klasörü, GitHub reposu…); başlık kuralı için öneri.
    pub word: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedGroup {
    /// `site:jira.togg.com` ya da `app:<uygulama kimliği>`; atama ve yoksayma bununla yapılır.
    pub key: String,
    pub kind: GroupKind,
    /// Site adı ya da uygulama adı.
    pub label: String,
    /// Grubun uygulaması (sitede en çok kullanılan tarayıcı).
    pub app_name: String,
    /// Kural deseni: alan adı ya da uygulama kimliği.
    pub pattern: String,
    pub seconds: i64,
    pub items: Vec<UnassignedItem>,
    /// Gösterilmeyen başlık sayısı.
    pub more: usize,
    /// Bu süreye en yakın zamanda çalışılan proje (öncesi ve sonrasındaki iş); yalnızca
    /// grubun en az yarısında aynı projeyse.
    pub likely_project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedIdle {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unassigned {
    /// Gruplardaki toplam (boşta süre hariç).
    pub total_seconds: i64,
    pub groups: Vec<UnassignedGroup>,
    /// Projeye atanmamış boşta aralıklar (bilgisayardan uzakta; çalışmaya sayılmaz).
    pub idle: Vec<UnassignedIdle>,
    pub idle_seconds: i64,
}

/// Oturumun grubu: (anahtar, tür, etiket, desen).
pub fn group_of(s: &Session) -> (String, GroupKind, String, String) {
    match s.domain.as_deref().filter(|d| !d.is_empty()) {
        Some(domain) if crate::browser::is_browser(&s.app_id) => (
            format!("site:{domain}"),
            GroupKind::Site,
            domain.to_string(),
            domain.to_string(),
        ),
        _ => (
            format!("app:{}", s.app_id),
            GroupKind::App,
            s.app_name.clone(),
            s.app_id.clone(),
        ),
    }
}

/// Listede gösterilen başlık: tarayıcı adı ve değişiklik işareti atılır.
pub fn item_title(s: &Session) -> String {
    let t = s.title.trim().trim_start_matches(['●', '•', '*']).trim();
    if crate::browser::is_browser(&s.app_id) {
        crate::browser::clean_title(t)
    } else {
        t.to_string()
    }
}

/// Başlık kuralı için öneri: tanınan proje adı (editör klasörü, GitHub reposu) ya da
/// başlıktaki iş anahtarının öneki (`LOY-214` → `LOY-`: projenin bütün işleri).
fn rule_word(s: &Session) -> Option<String> {
    crate::suggest::project_from_title(&s.app_id, &s.title, s.domain.as_deref()).or_else(|| {
        crate::timesheet::issue_keys(&s.title)
            .into_iter()
            .next()
            .and_then(|k| k.split_once('-').map(|(p, _)| format!("{p}-")))
    })
}

/// Atanmamış sayılan oturum mu? Projeye düşmeyen ve elle "Projesiz" denmemiş olan.
pub fn is_unassigned(s: &Session, classifier: &Classifier) -> bool {
    s.project_id.as_deref() != Some(NO_PROJECT) && classifier.classify(s).project.is_none()
}

/// `[from, to)` aralığının atanmamış süresi. `sessions` cihazlar arası birleştirilmiş,
/// başlangıca göre sıralı ve boşta kayıtlarını da içeren oturumlardır.
pub fn unassigned(
    sessions: &[Session],
    classifier: &Classifier,
    ignored: &HashSet<String>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Unassigned {
    // Süreler milisaniye olarak toplanır, en sonda saniyeye çevrilir (oturum başına kırpılmaz).
    let clip = |s: &Session| (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
    let projects: Vec<Option<String>> = sessions
        .iter()
        .map(|s| classifier.classify(s).project)
        .collect();
    let likely = neighbor_projects(sessions, &projects);

    struct Acc {
        kind: GroupKind,
        label: String,
        pattern: String,
        ms: i64,
        apps: HashMap<String, i64>,
        items: HashMap<String, (i64, Option<String>)>,
        votes: HashMap<String, i64>,
    }
    let mut groups: HashMap<String, Acc> = HashMap::new();
    let mut out = Unassigned::default();
    let mut idle_ms = 0;
    for (i, s) in sessions.iter().enumerate() {
        let ms = clip(s);
        if ms <= 0 || projects[i].is_some() || s.project_id.as_deref() == Some(NO_PROJECT) {
            continue;
        }
        if s.is_idle() {
            if ms / 1000 < MIN_IDLE_SECS {
                continue;
            }
            idle_ms += ms;
            out.idle.push(UnassignedIdle {
                start: s.started_at.max(from),
                end: s.ended_at.min(to),
                seconds: ms / 1000,
            });
            continue;
        }
        let (key, kind, label, pattern) = group_of(s);
        if ignored.contains(&key) {
            continue;
        }
        let acc = groups.entry(key).or_insert_with(|| Acc {
            kind,
            label,
            pattern,
            ms: 0,
            apps: HashMap::new(),
            items: HashMap::new(),
            votes: HashMap::new(),
        });
        acc.ms += ms;
        *acc.apps.entry(s.app_name.clone()).or_default() += ms;
        let item = acc
            .items
            .entry(item_title(s))
            .or_insert_with(|| (0, rule_word(s)));
        item.0 += ms;
        if let Some(p) = &likely[i] {
            *acc.votes.entry(p.clone()).or_default() += ms;
        }
    }

    let kept: Vec<(String, Acc)> = groups
        .into_iter()
        .filter(|(_, a)| a.ms / 1000 >= MIN_GROUP_SECS)
        .collect();
    out.total_seconds = kept.iter().map(|(_, a)| a.ms).sum::<i64>() / 1000;
    out.idle_seconds = idle_ms / 1000;
    let mut list: Vec<UnassignedGroup> = kept
        .into_iter()
        .map(|(key, a)| {
            let mut items: Vec<UnassignedItem> = a
                .items
                .into_iter()
                .map(|(title, (ms, word))| UnassignedItem {
                    title,
                    seconds: ms / 1000,
                    word,
                })
                .collect();
            items.sort_by(|x, y| y.seconds.cmp(&x.seconds).then(x.title.cmp(&y.title)));
            let total = items.len();
            items.retain(|i| i.seconds >= MIN_ITEM_SECS);
            items.truncate(MAX_ITEMS);
            let more = total - items.len();
            let likely_project = a
                .votes
                .into_iter()
                .max_by(|x, y| x.1.cmp(&y.1).then(y.0.cmp(&x.0)))
                .filter(|(_, v)| v * 2 >= a.ms)
                .map(|(p, _)| p);
            let app_name = a
                .apps
                .into_iter()
                .max_by(|x, y| x.1.cmp(&y.1).then(y.0.cmp(&x.0)))
                .map(|(n, _)| n)
                .unwrap_or_default();
            UnassignedGroup {
                key,
                kind: a.kind,
                label: a.label,
                app_name,
                pattern: a.pattern,
                seconds: a.ms / 1000,
                items,
                more,
                likely_project,
            }
        })
        .collect();
    list.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.label.cmp(&b.label)));
    out.groups = list;
    out
}

/// Her oturum için öncesinde ve sonrasında (`NEIGHBOR_GAP` içinde) çalışılan proje. İki yanda
/// farklı proje varsa daha yakın olan; atanmış oturumlar için `None`.
fn neighbor_projects(sessions: &[Session], projects: &[Option<String>]) -> Vec<Option<String>> {
    let n = sessions.len();
    // (proje, bitiş) — soldan en son biten atanmış oturum.
    let mut before: Vec<Option<(&String, DateTime<Utc>)>> = vec![None; n];
    let mut last: Option<(&String, DateTime<Utc>)> = None;
    for i in 0..n {
        before[i] = last;
        if let Some(p) = &projects[i]
            && last.is_none_or(|(_, end)| sessions[i].ended_at >= end)
        {
            last = Some((p, sessions[i].ended_at));
        }
    }
    let mut out = vec![None; n];
    let mut next: Option<(&String, DateTime<Utc>)> = None;
    for i in (0..n).rev() {
        if projects[i].is_none() {
            let s = &sessions[i];
            let gap_before = before[i].map(|(p, end)| (p, s.started_at - end));
            let gap_after = next.map(|(p, start)| (p, start - s.ended_at));
            out[i] = [gap_before, gap_after]
                .into_iter()
                .flatten()
                .filter(|(_, gap)| *gap <= NEIGHBOR_GAP)
                .min_by_key(|(_, gap)| *gap)
                .map(|(p, _)| p.clone());
        }
        if let Some(p) = &projects[i] {
            next = Some((p, sessions[i].started_at));
        }
    }
    out
}

/// Bir kuralın önceden hesaplanan etkisi.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulePreview {
    /// Kurala uyan toplam süre (öncelik ve elle atamadan bağımsız).
    pub matched_seconds: i64,
    /// Kural eklenince etikete geçecek, şu an etiketsiz süre.
    pub gained_seconds: i64,
    /// Kural eklenince başka bir etiketten bu etikete geçecek süre.
    pub taken_seconds: i64,
    /// Zaten bu etikette olan süre.
    pub already_seconds: i64,
    /// Uysa da elle atama ya da daha öncelikli kural yüzünden değişmeyecek süre.
    pub blocked_seconds: i64,
    /// Başka etiketten alınacak süre, etiket başına.
    pub taken_from: Vec<PreviewTag>,
    /// Etikete geçecek pencerelerden en uzunları.
    pub samples: Vec<PreviewSample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTag {
    pub id: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSample {
    pub app_name: String,
    pub title: String,
    pub seconds: i64,
}

const MAX_SAMPLES: usize = 8;

/// `rule` eklenseydi `[from, to)` aralığında ne değişirdi? `rules` mevcut kurallardır.
pub fn preview_rule(
    sessions: &[Session],
    tags: &[Tag],
    rules: &[Rule],
    rule: &Rule,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> RulePreview {
    let Some(kind) = tags.iter().find(|t| t.id == rule.tag_id).map(|t| t.kind) else {
        return RulePreview::default();
    };
    let before = Classifier::new(tags, rules);
    let mut with: Vec<Rule> = rules.to_vec();
    with.push(rule.clone());
    let after = Classifier::new(tags, &with);
    let pick = |c: &Classifier, s: &Session| {
        let class = c.classify(s);
        match kind {
            TagKind::Project => class.project,
            TagKind::Category => class.category,
        }
    };
    let pattern = rule.prepared_pattern();
    // Toplamlar önce milisaniye tutulur, en sonda saniyeye çevrilir (oturum başına kırpılmaz).
    let mut out = RulePreview::default();
    let mut taken: HashMap<String, i64> = HashMap::new();
    let mut samples: HashMap<(String, String), i64> = HashMap::new();
    for s in sessions {
        let ms = (s.ended_at.min(to) - s.started_at.max(from)).num_milliseconds();
        if ms <= 0 || !rule.matches_session(&pattern, s) {
            continue;
        }
        out.matched_seconds += ms;
        let (was, now) = (pick(&before, s), pick(&after, s));
        let target = Some(&rule.tag_id);
        if was.as_ref() == target {
            out.already_seconds += ms;
        } else if now.as_ref() != target {
            out.blocked_seconds += ms;
        } else {
            match was {
                Some(other) => {
                    out.taken_seconds += ms;
                    *taken.entry(other).or_default() += ms;
                }
                None => out.gained_seconds += ms,
            }
            *samples
                .entry((s.app_name.clone(), item_title(s)))
                .or_default() += ms;
        }
    }
    for v in [
        &mut out.matched_seconds,
        &mut out.gained_seconds,
        &mut out.taken_seconds,
        &mut out.already_seconds,
        &mut out.blocked_seconds,
    ] {
        *v /= 1000;
    }
    out.taken_from = taken
        .into_iter()
        .map(|(id, ms)| PreviewTag {
            id,
            seconds: ms / 1000,
        })
        .collect();
    out.taken_from
        .sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.id.cmp(&b.id)));
    let mut samples: Vec<PreviewSample> = samples
        .into_iter()
        .map(|((app_name, title), ms)| PreviewSample {
            app_name,
            title,
            seconds: ms / 1000,
        })
        .collect();
    samples.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.title.cmp(&b.title)));
    samples.truncate(MAX_SAMPLES);
    out.samples = samples;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::RuleField;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t(min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 2, 9, 0, 0).unwrap() + Duration::minutes(min)
    }

    fn s(app: &str, title: &str, domain: Option<&str>, from: i64, to: i64) -> Session {
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: app.rsplit('.').next().unwrap_or(app).into(),
            title: title.into(),
            url: domain.map(|d| format!("https://{d}/x")),
            domain: domain.map(Into::into),
            started_at: t(from),
            ended_at: t(to),
            category_id: None,
            project_id: None,
            block_from: None,
        }
    }

    fn tag(id: &str, kind: TagKind) -> Tag {
        Tag {
            id: id.into(),
            kind,
            name: id.into(),
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

    #[test]
    fn groups_unassigned_time_by_site_and_app_and_guesses_the_project() {
        let tags = [tag("kum", TagKind::Project)];
        let rules = [rule("kum", RuleField::Title, "kum")];
        let classifier = Classifier::new(&tags, &rules);
        let mut no_project = s("com.tinyspeck.slackmacgap", "Genel", None, 120, 125);
        no_project.project_id = Some(NO_PROJECT.into());
        let mut idle = Session::idle(t(200), t(230));
        idle.app_name = "Boşta".into();
        let sessions = vec![
            s("com.microsoft.VSCode", "main.rs — kum", None, 0, 30),
            s("com.tinyspeck.slackmacgap", "Genel", None, 32, 40),
            s(
                "com.google.Chrome",
                "PROJ-1 Ödeme - Jira - Google Chrome",
                Some("jira.togg.com"),
                41,
                61,
            ),
            s(
                "com.google.Chrome",
                "PROJ-2 Sepet - Google Chrome",
                Some("jira.togg.com"),
                61,
                66,
            ),
            s("com.microsoft.VSCode", "lib.rs — kum", None, 66, 90),
            no_project,
            s("com.apple.mail", "Gelen", None, 300, 300),
            idle,
        ];
        let out = unassigned(&sessions, &classifier, &HashSet::new(), t(0), t(24 * 60));
        // Slack (8 dk) 15 dakikadan kısa: önerilmez.
        assert_eq!(out.groups.len(), 1);
        let jira = &out.groups[0];
        assert_eq!(jira.key, "site:jira.togg.com");
        assert_eq!(jira.kind, GroupKind::Site);
        assert_eq!(jira.pattern, "jira.togg.com");
        assert_eq!(jira.seconds, 25 * 60);
        assert_eq!(jira.items[0].title, "PROJ-1 Ödeme - Jira");
        // 5 dakikalık başlık ayrı satır olmaz; grubu atamak onu da kapsar.
        assert_eq!((jira.items.len(), jira.more), (1, 1));
        assert_eq!(jira.items[0].word.as_deref(), Some("PROJ-"));
        // Jira, kum'da çalışılan iki oturumun arasında (biri 1 dk, diğeri 0 dk uzakta).
        assert_eq!(jira.likely_project.as_deref(), Some("kum"));
        assert_eq!(out.total_seconds, 25 * 60);
        assert_eq!(out.idle_seconds, 30 * 60);
        assert_eq!(out.idle.len(), 1);

        // Yoksayılan grup listelenmez.
        let ignored = HashSet::from(["site:jira.togg.com".to_string()]);
        let out = unassigned(&sessions, &classifier, &ignored, t(0), t(24 * 60));
        assert!(out.groups.is_empty());
        // Aralık dışına taşan kısım sayılmaz; kırpılınca 15 dakikanın altına inen grup düşer.
        let out = unassigned(&sessions, &classifier, &HashSet::new(), t(45), t(70));
        assert_eq!(out.groups[0].seconds, 21 * 60);
        let out = unassigned(&sessions, &classifier, &HashSet::new(), t(36), t(45));
        assert!(out.groups.is_empty());

        // Kısa boşta süre de önerilmez.
        let short = vec![Session::idle(t(0), t(10))];
        let out = unassigned(&short, &classifier, &HashSet::new(), t(0), t(60));
        assert_eq!((out.idle_seconds, out.idle.len()), (0, 0));
    }

    #[test]
    fn far_away_work_is_not_a_likely_project() {
        let tags = [tag("kum", TagKind::Project)];
        let rules = [rule("kum", RuleField::Title, "kum")];
        let classifier = Classifier::new(&tags, &rules);
        let sessions = vec![
            s("com.microsoft.VSCode", "main.rs — kum", None, 0, 30),
            s("com.apple.mail", "Gelen", None, 60, 80),
        ];
        let out = unassigned(&sessions, &classifier, &HashSet::new(), t(0), t(100));
        assert_eq!(out.groups[0].likely_project, None);
    }

    #[test]
    fn previews_what_a_rule_would_change() {
        let tags = [tag("kum", TagKind::Project), tag("togg", TagKind::Project)];
        let rules = [rule("togg", RuleField::Title, "togg")];
        let mut manual = s("com.apple.Safari", "kum togg", None, 50, 60);
        manual.project_id = Some("togg".into());
        let sessions = vec![
            s("com.microsoft.VSCode", "main.rs — kum", None, 0, 30),
            s(
                "com.google.Chrome",
                "kum togg - Google Chrome",
                None,
                30,
                40,
            ),
            manual,
            s("com.apple.mail", "Gelen", None, 60, 80),
        ];
        let new = rule("kum", RuleField::Title, "KUM");
        let p = preview_rule(&sessions, &tags, &rules, &new, t(0), t(100));
        assert_eq!(p.matched_seconds, 50 * 60);
        assert_eq!(p.gained_seconds, 30 * 60);
        // "kum togg": togg kuralı önce eklendi ve aynı türde; değişmez. Elle atanan da değişmez.
        assert_eq!(p.taken_seconds, 0);
        assert_eq!(p.blocked_seconds, 20 * 60);
        assert_eq!(p.samples[0].title, "main.rs — kum");

        // Site kuralı başlık kuralından önce gelir: togg'dan alır.
        let site = rule("kum", RuleField::Domain, "github.com");
        let mut sessions = sessions;
        sessions[1].url = Some("https://github.com/kum".into());
        let p = preview_rule(&sessions, &tags, &rules, &site, t(0), t(100));
        assert_eq!(p.taken_seconds, 10 * 60);
        assert_eq!(p.taken_from[0].id, "togg");
    }

    #[test]
    fn preview_folds_turkish_i_and_sums_milliseconds() {
        let tags = [tag("ist", TagKind::Project)];
        // Dört 1,5 saniyelik oturum: 6 sn (oturum başına kırpılsa 4 olurdu).
        let sessions: Vec<Session> = (0..4)
            .map(|i| {
                let mut x = s("com.apple.mail", "İstanbul Ofis", None, i, i);
                x.ended_at = x.started_at + Duration::milliseconds(1500);
                x
            })
            .collect();
        let new = rule("ist", RuleField::Title, "istanbul");
        let p = preview_rule(&sessions, &tags, &[], &new, t(0), t(100));
        assert_eq!((p.matched_seconds, p.gained_seconds), (6, 6));
        assert_eq!(p.samples[0].seconds, 6);
        // Boşta süre de milisaniyeyle toplanır: 4 × (15 dk + 1,5 sn).
        let idle: Vec<Session> = (0..4)
            .map(|i| {
                let start = t(i * 20);
                Session::idle(start, start + Duration::milliseconds(900_000 + 1500))
            })
            .collect();
        let out = unassigned(
            &idle,
            &Classifier::new(&tags, &[]),
            &HashSet::new(),
            t(0),
            t(100),
        );
        assert_eq!(out.idle_seconds, 3606);
    }
}
