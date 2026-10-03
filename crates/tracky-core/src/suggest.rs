//! Otomatik öneriler: pencere başlıklarından proje adları, bilinen uygulama ve
//! sitelerden kategoriler. Hiçbir şey kendiliğinden uygulanmaz; kullanıcı onaylarsa
//! sıradan bir etiket + kural olarak eklenir. Tamamen yerel çalışır.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::browser::is_browser;
use crate::classify::{
    Classifier, Rule, RuleField, Tag, TagKind, app_matches, default_category_id,
};
use crate::model::Session;

/// Bundan az süren proje / kategori önerilmez.
pub const MIN_SUGGEST_SECS: i64 = 30 * 60;
/// Her türden en fazla bu kadar öneri.
pub const MAX_SUGGESTIONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSuggestion {
    /// Yoksayma anahtarı.
    pub key: String,
    pub name: String,
    pub seconds: i64,
    /// Adın görüldüğü uygulamalar (en çok süreden aza).
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySuggestion {
    /// Yoksayma anahtarı.
    pub key: String,
    pub category_id: String,
    /// Oluşturulacak kural.
    pub field: RuleField,
    pub pattern: String,
    /// Kullanıcıya gösterilecek ad (uygulama ya da site).
    pub label: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestions {
    pub projects: Vec<ProjectSuggestion>,
    pub categories: Vec<CategorySuggestion>,
}

pub fn project_key(name: &str) -> String {
    format!("project:{}", name.to_lowercase())
}

/// Varsayılan kurallarda olmayan, tanınan uygulamalar: (varsayılan kategori, desenler).
const KNOWN_APPS: &[(&str, &[&str])] = &[
    (
        "Geliştirme",
        &[
            "com.microsoft.VSCodeInsiders",
            "com.exafunction.windsurf",
            "Windsurf.exe",
            "dev.zed.Zed",
            "com.sublimetext.*",
            "sublime_text.exe",
            "com.google.android.studio",
            "studio64.exe",
            "com.postmanlabs.mac",
            "Postman.exe",
            "com.docker.docker",
            "Docker Desktop.exe",
            "com.mitchellh.ghostty",
            "net.kovidgoyal.kitty",
            "io.alacritty",
            "alacritty.exe",
            "com.tableplus.TablePlus",
            "TablePlus.exe",
            "com.sequel-ace.sequel-ace",
            "com.linear",
            "Linear.exe",
            "com.github.wez.wezterm",
            "com.axosoft.gitkraken",
            "com.fork.Fork",
            "com.torusknot.SourceTreeNotMAS",
        ],
    ),
    (
        "İletişim",
        &[
            "com.microsoft.teams",
            "Teams.exe",
            "com.readdle.smartemail-Mac",
            "com.superhuman.electron",
            "com.webex.meetingmanager",
            "Webex.exe",
            "com.skype.skype",
            "Skype.exe",
            "com.facebook.archon",
            "Messenger.exe",
            "com.apple.iChat",
            "org.whispersystems.signal-desktop",
            "Signal.exe",
        ],
    ),
    (
        "Tasarım",
        &[
            "com.adobe.xd",
            "XD.exe",
            "com.pixelmatorteam.pixelmator.x",
            "com.framer.electron",
            "org.blenderfoundation.blender",
            "blender.exe",
            "com.adobe.InDesign",
            "InDesign.exe",
            "com.adobe.LightroomClassicCC7",
            "Lightroom.exe",
        ],
    ),
    (
        "Belgeler",
        &[
            "com.microsoft.onenote.mac",
            "ONENOTE.EXE",
            "com.apple.TextEdit",
            "notepad.exe",
            "com.readdle.PDFExpert-Mac",
            "com.adobe.Acrobat.Pro",
            "com.adobe.Reader",
            "Acrobat.exe",
            "AcroRd32.exe",
            "com.culturedcode.ThingsMac",
            "com.todoist.mac.Todoist",
            "Todoist.exe",
            "com.apple.reminders",
            "com.agiletortoise.Drafts-OSX",
            "net.shinyfrog.bear",
        ],
    ),
    (
        "Sosyal & Eğlence",
        &[
            "com.valvesoftware.steam",
            "steam.exe",
            "com.apple.podcasts",
            "com.apple.news",
            "com.netflix.Netflix",
            "com.apple.Photos",
        ],
    ),
];

/// Tarayıcıda tanınan siteler: (varsayılan kategori, [(alan adı, başlıkta geçen ad)]).
/// Kural başlıktaki adla kurulur; alan adı yalnızca tespiti güçlendirir. Başlık kuralı her
/// uygulamada eşleştiği için sıradan sözcük olan adlar (Zoom, Linear…) listede yok.
const KNOWN_SITES: &[(&str, &[(&str, &str)])] = &[
    (
        "Geliştirme",
        &[
            ("gitlab.com", "GitLab"),
            ("bitbucket.org", "Bitbucket"),
            ("vercel.com", "Vercel"),
            ("supabase.com", "Supabase"),
            ("developer.mozilla.org", "MDN"),
            ("atlassian.net", "Jira"),
            ("netlify.com", "Netlify"),
            ("console.cloud.google.com", "Google Cloud"),
            ("crates.io", "crates.io"),
        ],
    ),
    (
        "İletişim",
        &[
            ("app.slack.com", "Slack"),
            ("teams.microsoft.com", "Microsoft Teams"),
            ("discord.com", "Discord"),
            ("web.telegram.org", "Telegram"),
        ],
    ),
    ("Tasarım", &[("miro.com", "Miro"), ("framer.com", "Framer")]),
    (
        "Belgeler",
        &[
            ("atlassian.net/wiki", "Confluence"),
            ("airtable.com", "Airtable"),
            ("trello.com", "Trello"),
            ("drive.google.com", "Google Drive"),
            ("onedrive.live.com", "OneDrive"),
        ],
    ),
    (
        "Sosyal & Eğlence",
        &[
            ("linkedin.com", "LinkedIn"),
            ("primevideo.com", "Prime Video"),
            ("disneyplus.com", "Disney+"),
            ("open.spotify.com", "Spotify"),
            ("pinterest.com", "Pinterest"),
            ("threads.net", "Threads"),
            ("bsky.app", "Bluesky"),
        ],
    ),
];

/// Proje adı sayılmayan başlık parçaları (küçük harf).
const NOT_PROJECTS: &[&str] = &[
    "untitled",
    "untitled (workspace)",
    "welcome",
    "hoş geldiniz",
    "settings",
    "ayarlar",
    "extensions",
    "uzantılar",
    "home",
    "new tab",
    "yeni sekme",
    "zsh",
    "-zsh",
    "bash",
    "-bash",
    "fish",
    "powershell",
    "cmd",
    "~",
    "github",
    "visual studio code",
    "cursor",
    "xcode",
    "terminal",
];

fn lower_app(app_id: &str) -> String {
    app_id.to_lowercase()
}

fn any_app(id: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| app_matches(&p.to_lowercase(), id))
}

/// Başlıktan proje adı: bilinen editör, IDE, terminal ve GitHub kalıpları.
pub fn project_from_title(app_id: &str, title: &str, domain: Option<&str>) -> Option<String> {
    let id = lower_app(app_id);
    let title = title.trim().trim_start_matches(['●', '•', '*']).trim();
    let name = if any_app(
        &id,
        &[
            "com.microsoft.vscode*",
            "code.exe",
            "com.todesktop.230313mzl4w4u92",
            "cursor.exe",
            "com.exafunction.windsurf",
            "windsurf.exe",
            "dev.zed.zed",
        ],
    ) {
        // "dosya — proje" (macOS) ya da "dosya - proje - Visual Studio Code" (Windows).
        let parts: Vec<&str> = if title.contains(" — ") {
            title.split(" — ").collect()
        } else {
            title.split(" - ").collect()
        };
        let parts: Vec<&str> = parts
            .into_iter()
            .map(str::trim)
            .filter(|p| {
                !matches!(
                    p.to_lowercase().as_str(),
                    "visual studio code" | "cursor" | "windsurf" | "zed"
                )
            })
            .collect();
        (parts.len() >= 2).then(|| parts[parts.len() - 1])
    } else if any_app(
        &id,
        &[
            "com.jetbrains.*",
            "idea64.exe",
            "pycharm64.exe",
            "webstorm64.exe",
            "rider64.exe",
            "goland64.exe",
            "clion64.exe",
        ],
    ) {
        // "proje – dosya" (en tire) ya da "proje [yol] – dosya".
        title.split(" – ").next().filter(|_| title.contains(" – "))
    } else if any_app(&id, &["com.apple.dt.xcode"]) {
        title.split(" — ").next().filter(|_| title.contains(" — "))
    } else if any_app(
        &id,
        &[
            "com.apple.terminal",
            "com.googlecode.iterm2",
            "dev.warp.warp-stable",
            "com.mitchellh.ghostty",
        ],
    ) {
        // "klasör — -zsh — 80×24" ya da "kullanıcı@makine:~/yol/klasör".
        let first = title.split(" — ").next().unwrap_or(title);
        let path = first.rsplit(':').next().unwrap_or(first);
        path.trim().trim_end_matches('/').rsplit('/').next()
    } else if is_browser(&id)
        && (domain.is_some_and(|d| d.ends_with("github.com"))
            || title.to_lowercase().contains("github"))
    {
        github_repo(title)
    } else {
        None
    }?;
    let name = clean_project(name)?;
    Some(name)
}

/// GitHub sayfa başlığındaki "sahip/repo" içinden repo adı.
fn github_repo(title: &str) -> Option<&str> {
    title
        .split([' ', ':', '·'])
        .map(|w| w.trim())
        .find_map(|w| {
            let (owner, repo) = w.split_once('/')?;
            let ok = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            };
            (ok(owner) && ok(repo) && !repo.contains('/')).then_some(repo)
        })
}

/// Parantezli ekleri atar ("tracky (Workspace)", "proje [~/kod/proje]"), gürültüyü eler.
fn clean_project(name: &str) -> Option<String> {
    let mut name = name.trim();
    for open in [" (", " ["] {
        if let Some(i) = name.find(open) {
            name = name[..i].trim();
        }
    }
    let lower = name.to_lowercase();
    let looks_like_file = name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    let valid = (2..=40).contains(&name.chars().count())
        && !NOT_PROJECTS.contains(&lower.as_str())
        && !looks_like_file
        && !name.starts_with('/')
        && name.chars().any(char::is_alphanumeric);
    valid.then(|| name.to_string())
}

fn secs(s: &Session) -> i64 {
    (s.ended_at - s.started_at).num_seconds().max(0)
}

/// Oturumlardan öneriler. `dismissed`: kullanıcının yoksaydığı anahtarlar.
pub fn suggest(
    sessions: &[Session],
    tags: &[Tag],
    rules: &[Rule],
    dismissed: &HashSet<String>,
) -> Suggestions {
    let classifier = Classifier::new(tags, rules);
    Suggestions {
        projects: suggest_projects(sessions, tags, rules, &classifier, dismissed),
        categories: suggest_categories(sessions, tags, &classifier, dismissed),
    }
}

fn suggest_projects(
    sessions: &[Session],
    tags: &[Tag],
    rules: &[Rule],
    classifier: &Classifier,
    dismissed: &HashSet<String>,
) -> Vec<ProjectSuggestion> {
    // Zaten bir projeye düşen ya da bir projenin adını/desenini taşıyan adlar önerilmez.
    let known: Vec<String> = tags
        .iter()
        .filter(|t| t.kind == TagKind::Project)
        .map(|t| t.name.to_lowercase())
        .chain(
            rules
                .iter()
                .filter(|r| {
                    r.field == RuleField::Title
                        && tags
                            .iter()
                            .any(|t| t.id == r.tag_id && t.kind == TagKind::Project)
                })
                .map(|r| r.pattern.to_lowercase()),
        )
        .collect();

    // anahtar → (yazılışlar ve süreleri, uygulamalar ve süreleri, toplam)
    type Acc = (HashMap<String, i64>, HashMap<String, i64>, i64);
    let mut found: HashMap<String, Acc> = HashMap::new();
    for s in sessions {
        if s.is_manual() || classifier.classify(s).project.is_some() {
            continue;
        }
        let Some(name) = project_from_title(&s.app_id, &s.title, s.domain.as_deref()) else {
            continue;
        };
        let d = secs(s);
        let e = found.entry(name.to_lowercase()).or_default();
        *e.0.entry(name).or_default() += d;
        *e.1.entry(s.app_name.clone()).or_default() += d;
        e.2 += d;
    }

    let mut out: Vec<ProjectSuggestion> = found
        .into_iter()
        .filter(|(key, acc)| {
            acc.2 >= MIN_SUGGEST_SECS
                && !dismissed.contains(&format!("project:{key}"))
                && !known.iter().any(|k| k == key || key.contains(k.as_str()))
        })
        .map(|(key, (spellings, apps, total))| {
            let name = top_by_secs(spellings)
                .into_iter()
                .next()
                .unwrap_or(key.clone());
            ProjectSuggestion {
                key: format!("project:{key}"),
                name,
                seconds: total,
                apps: top_by_secs(apps),
            }
        })
        .collect();
    out.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.name.cmp(&b.name)));
    out.truncate(MAX_SUGGESTIONS);
    out
}

fn top_by_secs(map: HashMap<String, i64>) -> Vec<String> {
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(k, _)| k).collect()
}

fn suggest_categories(
    sessions: &[Session],
    tags: &[Tag],
    classifier: &Classifier,
    dismissed: &HashSet<String>,
) -> Vec<CategorySuggestion> {
    // Yalnızca hâlâ var olan varsayılan kategorilere öneri (adı değişmiş olabilir).
    let category = |name: &str| {
        let id = default_category_id(name);
        tags.iter()
            .any(|t| t.id == id && t.kind == TagKind::Category)
            .then_some(id)
    };
    let mut found: HashMap<String, CategorySuggestion> = HashMap::new();
    for s in sessions {
        if s.is_manual() || classifier.classify(s).category.is_some() {
            continue;
        }
        let id = lower_app(&s.app_id);
        let hit = if is_browser(&id) {
            let title = s.title.to_lowercase();
            KNOWN_SITES.iter().find_map(|(cat, sites)| {
                sites.iter().find_map(|(domain, name)| {
                    let by_domain = s.domain.as_deref().is_some_and(|d| {
                        let site = domain.split('/').next().unwrap_or(domain);
                        d == site || d.ends_with(&format!(".{site}"))
                    });
                    (by_domain || title.contains(&name.to_lowercase())).then(|| {
                        (
                            *cat,
                            RuleField::Title,
                            name.to_string(),
                            name.to_string(),
                            format!("site:{}", name.to_lowercase()),
                        )
                    })
                })
            })
        } else {
            KNOWN_APPS
                .iter()
                .find(|(_, apps)| any_app(&id, apps))
                .map(|(cat, _)| {
                    (
                        *cat,
                        RuleField::App,
                        s.app_id.clone(),
                        s.app_name.clone(),
                        format!("app:{id}"),
                    )
                })
        };
        let Some((cat, field, pattern, label, key)) = hit else {
            continue;
        };
        let Some(category_id) = category(cat) else {
            continue;
        };
        if dismissed.contains(&key) {
            continue;
        }
        found
            .entry(key.clone())
            .or_insert(CategorySuggestion {
                key,
                category_id,
                field,
                pattern,
                label,
                seconds: 0,
            })
            .seconds += secs(s);
    }
    let mut out: Vec<CategorySuggestion> = found
        .into_values()
        .filter(|c| c.seconds >= MIN_SUGGEST_SECS / 3)
        .collect();
    out.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.label.cmp(&b.label)));
    out.truncate(MAX_SUGGESTIONS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use uuid::Uuid;

    fn p(app: &str, title: &str) -> Option<String> {
        project_from_title(app, title, None)
    }

    #[test]
    fn projects_from_editor_ide_terminal_and_github_titles() {
        assert_eq!(
            p("com.microsoft.VSCode", "sync.rs — tracky").as_deref(),
            Some("tracky")
        );
        assert_eq!(
            p("com.microsoft.VSCode", "● sync.rs — tracky (Workspace)").as_deref(),
            Some("tracky")
        );
        assert_eq!(
            p(
                r"C:\Program Files\Microsoft VS Code\Code.exe",
                "lib.rs - fintrack - Visual Studio Code"
            )
            .as_deref(),
            Some("fintrack")
        );
        assert_eq!(
            p("com.microsoft.VSCode", "Welcome — tracky").as_deref(),
            Some("tracky")
        );
        assert_eq!(p("com.microsoft.VSCode", "Settings"), None);
        assert_eq!(
            p("com.microsoft.VSCode", "notes.md — Untitled (Workspace)"),
            None
        );
        assert_eq!(
            p(
                "com.jetbrains.intellij",
                "kum-api [~/code/kum-api] – Main.kt"
            )
            .as_deref(),
            Some("kum-api")
        );
        assert_eq!(
            p("com.apple.dt.Xcode", "Kum — ContentView.swift").as_deref(),
            Some("Kum")
        );
        assert_eq!(
            p("com.apple.Terminal", "tracky — -zsh — 80×24").as_deref(),
            Some("tracky")
        );
        assert_eq!(p("com.apple.Terminal", "~ — -zsh — 80×24"), None);
        assert_eq!(
            p("com.googlecode.iterm2", "kaan@mac:~/code/fintrack-os").as_deref(),
            Some("fintrack-os")
        );
        assert_eq!(
            project_from_title(
                "com.google.Chrome",
                "Fix sync · Pull Request #12 · mylifer/tracky",
                Some("github.com")
            )
            .as_deref(),
            Some("tracky")
        );
        assert_eq!(
            p("com.google.Chrome", "GitHub - mylifer/tracky: Zaman takibi").as_deref(),
            Some("tracky")
        );
        assert_eq!(p("com.google.Chrome", "YouTube"), None);
        assert_eq!(p("com.apple.mail", "Re: tracky — Gelen Kutusu"), None);
    }

    fn session(app: &str, name: &str, title: &str, mins: i64) -> Session {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        Session {
            id: Uuid::new_v4(),
            app_id: app.into(),
            app_name: name.into(),
            title: title.into(),
            url: None,
            domain: None,
            started_at: t0,
            ended_at: t0 + Duration::minutes(mins),
            category_id: None,
        }
    }

    fn defaults() -> Vec<Tag> {
        crate::classify::DEFAULT_CATEGORIES
            .iter()
            .map(|(name, color, _, _)| Tag {
                id: default_category_id(name),
                kind: TagKind::Category,
                name: name.to_string(),
                color: *color,
            })
            .collect()
    }

    #[test]
    fn suggests_projects_across_apps_and_respects_existing_and_dismissed() {
        let sessions = vec![
            session("com.microsoft.VSCode", "Code", "sync.rs — tracky", 25),
            session(
                "com.apple.Terminal",
                "Terminal",
                "tracky — -zsh — 80×24",
                10,
            ),
            session("com.microsoft.VSCode", "Code", "a.rs — kisa", 5),
        ];
        let s = suggest(&sessions, &defaults(), &[], &HashSet::new());
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].name, "tracky");
        assert_eq!(s.projects[0].seconds, 35 * 60);
        assert_eq!(s.projects[0].apps, ["Code", "Terminal"]);

        let dismissed = HashSet::from([project_key("Tracky")]);
        assert!(
            suggest(&sessions, &defaults(), &[], &dismissed)
                .projects
                .is_empty()
        );

        let mut tags = defaults();
        tags.push(Tag {
            id: "p1".into(),
            kind: TagKind::Project,
            name: "Kum".into(),
            color: 2,
        });
        let rules = [Rule {
            id: "r1".into(),
            tag_id: "p1".into(),
            field: RuleField::Title,
            pattern: "tracky".into(),
        }];
        assert!(
            suggest(&sessions, &tags, &rules, &HashSet::new())
                .projects
                .is_empty()
        );
    }

    #[test]
    fn suggests_categories_for_known_uncategorized_apps_and_sites() {
        let mut linkedin = session("com.google.Chrome", "Google Chrome", "Akış | LinkedIn", 12);
        linkedin.domain = Some("www.linkedin.com".into());
        let sessions = vec![
            session("com.postmanlabs.mac", "Postman", "Kum API", 15),
            linkedin,
            session("com.microsoft.VSCode", "Code", "x — y", 60), // zaten Geliştirme
            session("com.unknown.App", "Bilinmeyen", "", 60),
        ];
        let tags = defaults();
        let rules = [Rule {
            id: "r".into(),
            tag_id: default_category_id("Geliştirme"),
            field: RuleField::App,
            pattern: "com.microsoft.VSCode".into(),
        }];
        let s = suggest(&sessions, &tags, &rules, &HashSet::new());
        let got: Vec<_> = s
            .categories
            .iter()
            .map(|c| (c.label.as_str(), c.field, c.pattern.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Postman", RuleField::App, "com.postmanlabs.mac"),
                ("LinkedIn", RuleField::Title, "LinkedIn")
            ]
        );
        assert_eq!(
            s.categories[0].category_id,
            default_category_id("Geliştirme")
        );
        assert_eq!(
            s.categories[1].category_id,
            default_category_id("Sosyal & Eğlence")
        );

        // Kullanıcı kategoriyi sildiyse o kategoriye öneri yok.
        let without_social: Vec<Tag> = tags
            .into_iter()
            .filter(|t| t.name != "Sosyal & Eğlence")
            .collect();
        assert_eq!(
            suggest(&sessions, &without_social, &rules, &HashSet::new())
                .categories
                .len(),
            1
        );
    }
}
