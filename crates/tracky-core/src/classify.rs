//! Kategoriler, projeler ve oturumları bunlara bağlayan kurallar.
//!
//! Sınıflandırma kayıt anında değil sorgu anında yapılır; böylece bir kural
//! değiştiğinde geçmiş veriler de yeni kurala göre raporlanır.

use serde::{Deserialize, Serialize};

use crate::model::Session;

/// Kategori bir uygulama grubudur (İletişim, Geliştirme...); proje ise
/// pencere başlığından tanınan, uygulamalar arası bir iştir (örn. "fintrack").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagKind {
    Category,
    Project,
}

impl TagKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TagKind::Category => "category",
            TagKind::Project => "project",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "category" => Some(TagKind::Category),
            "project" => Some(TagKind::Project),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub kind: TagKind,
    pub name: String,
    /// Kategorik paletteki sabit yuva (1-8). Renk sıraya göre değil varlığa göre atanır.
    pub color: u8,
}

/// Kuralın neye baktığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleField {
    /// Uygulama kimliği eşitliği (macOS bundle id ya da Windows exe adı/yolu).
    /// Sonu `*` ile biten desen önek eşleşmesidir (örn. `com.jetbrains.*`).
    App,
    /// Pencere başlığı deseni içeriyor mu (büyük/küçük harf duyarsız).
    Title,
}

impl RuleField {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleField::App => "app",
            RuleField::Title => "title",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app" => Some(RuleField::App),
            "title" => Some(RuleField::Title),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub tag_id: String,
    pub field: RuleField,
    pub pattern: String,
}

impl Rule {
    pub fn matches(&self, app_id: &str, title: &str) -> bool {
        self.matches_lower(
            &self.pattern.to_lowercase(),
            &app_id.to_lowercase(),
            &title.to_lowercase(),
        )
    }

    /// Desen ve girdiler önceden küçük harfe çevrilmiş olarak (sınıflandırıcı
    /// desenleri bir kez çevirir, her oturum için değil).
    fn matches_lower(&self, pattern: &str, app_id: &str, title: &str) -> bool {
        match self.field {
            RuleField::App => app_matches(pattern, app_id),
            RuleField::Title => !pattern.is_empty() && title.contains(pattern),
        }
    }
}

/// Windows'ta `app_id` tam exe yoludur; desen yalnızca exe adı da olabilir.
/// İkisi de küçük harfli gelir.
fn app_matches(pattern: &str, id: &str) -> bool {
    let exe = id.rsplit(['\\', '/']).next().unwrap_or(id);
    match pattern.strip_suffix('*') {
        Some(prefix) if !prefix.is_empty() => id.starts_with(prefix) || exe.starts_with(prefix),
        Some(_) => false,
        None => id == pattern || exe == pattern,
    }
}

/// Bir oturumun kategorisi ve projesi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classification {
    pub category: Option<String>,
    pub project: Option<String>,
}

/// Kuralları önceliğe göre dizer: başlık kuralları uygulama kurallarından
/// daha özeldir ve önce denenir (örn. Safari'de "Google E-Tablolar" → Ofis).
pub struct Classifier {
    /// (kural, küçük harfli desen)
    category_rules: Vec<(Rule, String)>,
    project_rules: Vec<(Rule, String)>,
    /// Var olan kategoriler: silinmiş bir kategoriye verilmiş elle atama yok sayılır.
    categories: std::collections::HashSet<String>,
}

impl Classifier {
    pub fn new(tags: &[Tag], rules: &[Rule]) -> Self {
        let kind_of = |tag_id: &str| tags.iter().find(|t| t.id == tag_id).map(|t| t.kind);
        let mut category_rules = Vec::new();
        let mut project_rules = Vec::new();
        for rule in rules {
            match kind_of(&rule.tag_id) {
                Some(TagKind::Category) => category_rules.push(rule.clone()),
                Some(TagKind::Project) => project_rules.push(rule.clone()),
                None => {}
            }
        }
        // Kararlı sıralama: aynı türdeki kuralların kendi sırası korunur.
        for list in [&mut category_rules, &mut project_rules] {
            list.sort_by_key(|r| r.field != RuleField::Title);
        }
        let lowered = |rules: Vec<Rule>| -> Vec<(Rule, String)> {
            rules
                .into_iter()
                .map(|r| {
                    let pattern = r.pattern.to_lowercase();
                    (r, pattern)
                })
                .collect()
        };
        Self {
            category_rules: lowered(category_rules),
            project_rules: lowered(project_rules),
            categories: tags
                .iter()
                .filter(|t| t.kind == TagKind::Category)
                .map(|t| t.id.clone())
                .collect(),
        }
    }

    /// Elle verilen kategori (hâlâ varsa) kurallardan önce gelir.
    pub fn classify(&self, session: &Session) -> Classification {
        let mut class = self.classify_parts(&session.app_id, &session.title);
        if let Some(id) = &session.category_id
            && self.categories.contains(id)
        {
            class.category = Some(id.clone());
        }
        class
    }

    pub fn classify_parts(&self, app_id: &str, title: &str) -> Classification {
        let (app_id, title) = (app_id.to_lowercase(), title.to_lowercase());
        let first = |rules: &[(Rule, String)]| {
            rules
                .iter()
                .find(|(r, p)| r.matches_lower(p, &app_id, &title))
                .map(|(r, _)| r.tag_id.clone())
        };
        Classification {
            category: first(&self.category_rules),
            project: first(&self.project_rules),
        }
    }

    /// Uygulamanın kendi kategorisi (yalnızca uygulama kuralları; listede göstermek için).
    pub fn app_category(&self, app_id: &str) -> Option<String> {
        let app_id = app_id.to_lowercase();
        self.category_rules
            .iter()
            .find(|(r, p)| r.field == RuleField::App && r.matches_lower(p, &app_id, ""))
            .map(|(r, _)| r.tag_id.clone())
    }
}

/// İlk açılışta eklenen kategoriler: (ad, renk yuvası, uygulama desenleri, başlık desenleri).
pub const DEFAULT_CATEGORIES: &[(&str, u8, &[&str], &[&str])] = &[
    (
        "Geliştirme",
        1,
        &[
            "com.microsoft.VSCode",
            "Code.exe",
            "com.todesktop.230313mzl4w4u92",
            "Cursor.exe",
            "com.apple.Terminal",
            "com.googlecode.iterm2",
            "dev.warp.Warp-Stable",
            "WindowsTerminal.exe",
            "powershell.exe",
            "cmd.exe",
            "com.apple.dt.Xcode",
            "com.jetbrains.*",
            "idea64.exe",
            "devenv.exe",
            "com.github.GitHubClient",
            "GitHubDesktop.exe",
        ],
        &["GitHub", "Stack Overflow", "localhost"],
    ),
    (
        "İletişim",
        2,
        &[
            "com.tinyspeck.slackmacgap",
            "slack.exe",
            "com.microsoft.teams2",
            "ms-teams.exe",
            "net.whatsapp.WhatsApp",
            "WhatsApp.exe",
            "com.apple.mail",
            "com.microsoft.Outlook",
            "OUTLOOK.EXE",
            "olk.exe",
            "ru.keepcoder.Telegram",
            "Telegram.exe",
            "com.hnc.Discord",
            "Discord.exe",
            "us.zoom.xos",
            "Zoom.exe",
            "com.apple.MobileSMS",
            "com.apple.FaceTime",
        ],
        &["Gmail", "Outlook", "WhatsApp", "Google Meet"],
    ),
    (
        "Tasarım",
        3,
        &[
            "com.figma.Desktop",
            "Figma.exe",
            "com.bohemiancoding.sketch3",
            "com.adobe.Photoshop*",
            "Photoshop.exe",
            "com.adobe.illustrator",
            "Illustrator.exe",
            "tw.ogdesign.eagle",
            "Eagle.exe",
            "com.seriflabs.*",
        ],
        &["Figma", "Canva", "Dribbble", "Behance"],
    ),
    (
        "Belgeler",
        4,
        &[
            "com.microsoft.Excel",
            "EXCEL.EXE",
            "com.microsoft.Word",
            "WINWORD.EXE",
            "com.microsoft.Powerpoint",
            "POWERPNT.EXE",
            "com.apple.iWork.*",
            "notion.id",
            "Notion.exe",
            "md.obsidian",
            "Obsidian.exe",
            "com.apple.Notes",
            "com.apple.Preview",
        ],
        &[
            "Google E-Tablolar",
            "Google Sheets",
            "Google Dokümanlar",
            "Google Docs",
            "Google Slaytlar",
            "Google Slides",
            "Notion",
        ],
    ),
    (
        "Sosyal & Eğlence",
        5,
        &[
            "com.spotify.client",
            "Spotify.exe",
            "com.apple.Music",
            "com.apple.TV",
        ],
        &[
            "/ X",
            "X'te ",
            "YouTube",
            "Instagram",
            "Facebook",
            "Reddit",
            "TikTok",
            "Netflix",
            "Twitch",
            "Ekşi Sözlük",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

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
            id: format!("{tag}-{pattern}"),
            tag_id: tag.into(),
            field,
            pattern: pattern.into(),
        }
    }

    #[test]
    fn app_patterns() {
        let r = rule("dev", RuleField::App, "Code.exe");
        assert!(r.matches(
            r"C:\Users\k\AppData\Local\Programs\Microsoft VS Code\Code.exe",
            ""
        ));
        assert!(r.matches("code.exe", ""));
        assert!(!r.matches("com.microsoft.VSCode", ""));
        let r = rule("dev", RuleField::App, "com.jetbrains.*");
        assert!(r.matches("com.jetbrains.intellij", ""));
        assert!(!r.matches("com.apple.Safari", ""));
        assert!(!rule("x", RuleField::App, "*").matches("anything", ""));
    }

    #[test]
    fn title_rules_win_over_app_rules_and_projects_are_independent() {
        let tags = [
            tag("browse", TagKind::Category),
            tag("docs", TagKind::Category),
            tag("fintrack", TagKind::Project),
        ];
        let rules = [
            rule("browse", RuleField::App, "com.apple.Safari"),
            rule("docs", RuleField::Title, "google e-tablolar"),
            rule("fintrack", RuleField::Title, "fintrack"),
            rule("ghost", RuleField::App, "com.apple.Safari"), // silinmiş etiket
        ];
        let c = Classifier::new(&tags, &rules);
        assert_eq!(
            c.classify_parts("com.apple.Safari", "Bütçe - Google E-Tablolar"),
            Classification {
                category: Some("docs".into()),
                project: None
            }
        );
        assert_eq!(
            c.classify_parts("com.apple.Safari", "fintrack-os PR #12"),
            Classification {
                category: Some("browse".into()),
                project: Some("fintrack".into())
            }
        );
        assert_eq!(
            c.app_category("com.apple.Safari").as_deref(),
            Some("browse")
        );
        assert_eq!(
            c.classify_parts("com.other", "x"),
            Classification::default()
        );
    }

    #[test]
    fn defaults_use_distinct_color_slots() {
        let mut slots: Vec<u8> = DEFAULT_CATEGORIES.iter().map(|c| c.1).collect();
        slots.dedup();
        assert_eq!(slots.len(), DEFAULT_CATEGORIES.len());
        assert!(slots.iter().all(|s| (1..=8).contains(s)));
    }
}
