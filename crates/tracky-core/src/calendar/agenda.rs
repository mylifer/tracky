//! Toplantı davetinin metninden gündem: Outlook'un yayımladığı DESCRIPTION (düz metin ya da
//! HTML) okunur; Teams/Zoom katılım bloğu, iletilen davetin üst bilgisi ve bağlantılar atılır.
//! Kalan, yapay zekâya açıklama yazarken kaynak olur ve toplantının ayrıntısında görünür.

/// Gündemin en uzun hali (karakter); uzun davet metni istemi ve arayüzü şişirmesin.
pub const MAX_AGENDA_CHARS: usize = 600;

/// Bu satırlardan biriyle başlayan yerden sonrası katılım bilgisidir (küçük harfle karşılaştırılır).
const JOIN_MARKERS: &[&str] = &[
    "microsoft teams",
    "join the meeting now",
    "join on your computer",
    "toplantıya şimdi katılın",
    "bilgisayarınızda, mobil uygulamada",
    "join zoom meeting",
    "zoom toplantısına katılın",
    "join with google meet",
    "google meet ile katılın",
    "join webex meeting",
];

/// İletilen davetin üst bilgisi: bu önekle başlayan satır atılır (küçük harfle karşılaştırılır).
const HEADER_PREFIXES: &[&str] = &[
    "from:",
    "sent:",
    "to:",
    "cc:",
    "subject:",
    "when:",
    "where:",
    "location:",
    "kimden:",
    "gönderildi:",
    "gönderilen:",
    "kime:",
    "bilgi:",
    "konu:",
    "ne zaman:",
    "nerede:",
    "konum:",
];

/// Davet metninden gündem; anlamlı bir şey kalmazsa boş.
pub fn agenda(raw: &str) -> String {
    let text = if looks_like_html(raw) {
        html_to_text(raw)
    } else {
        raw.to_string()
    };
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if is_separator(line) {
            // Teams bloğu "____" ile başlar; başta gelirse (davette başka metin yok) atlanır.
            if lines.is_empty() {
                continue;
            }
            break;
        }
        let lower = line.to_lowercase();
        if JOIN_MARKERS.iter().any(|m| lower.starts_with(m)) {
            break;
        }
        if HEADER_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            continue;
        }
        let line = strip_links(line);
        if !line.is_empty() && has_words(&line) {
            lines.push(line);
        }
    }
    truncate(&lines.join("\n"), MAX_AGENDA_CHARS)
}

fn looks_like_html(s: &str) -> bool {
    let lower = s.get(..s.len().min(400)).unwrap_or(s).to_ascii_lowercase();
    [
        "<html", "<body", "<div", "<p>", "<p ", "<br", "<span", "<table",
    ]
    .iter()
    .any(|t| lower.contains(t))
}

/// Ayırıcı çizgi: "_____" (Teams), "-----Original Appointment-----" (iletilen davet).
fn is_separator(line: &str) -> bool {
    line.starts_with("_____") || line.starts_with("-----") || line.starts_with("─────")
}

/// En az bir harf ya da rakam (yalnızca noktalama kalmış satır atılır).
fn has_words(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// HTML'den düz metin: blok etiketleri satır sonu olur, `<head>`, `<style>` ve `<script>`
/// içeriği atılır, öteki etiketler silinir, karakter varlıkları çözülür.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            rest = "";
            break;
        };
        let tag = after[..close].trim().to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        rest = &after[close + 1..];
        if !tag.starts_with('/') && matches!(name.as_str(), "head" | "style" | "script") {
            // Kapanış etiketine kadar atlanır (yoksa sonuna kadar).
            let end = format!("</{name}");
            rest = match rest.to_ascii_lowercase().find(&end) {
                Some(i) => rest[i..].find('>').map_or("", |j| &rest[i + j + 1..]),
                None => "",
            };
            continue;
        }
        if matches!(
            name.as_str(),
            "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "table" | "hr"
        ) {
            out.push('\n');
        } else if matches!(name.as_str(), "td" | "th") {
            out.push(' ');
        }
    }
    out.push_str(rest);
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let decoded = after.find(';').filter(|&j| j <= 10).and_then(|j| {
            let name = &after[..j];
            let c = match name {
                "nbsp" => Some(' '),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name.strip_prefix('#').and_then(|n| {
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            c.map(|c| (c, j))
        });
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &after[j + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Bağlantılar atılır: `<https://…>` (Outlook düz metninde bağlantı metninin yanında), çıplak
/// adresler ve `mailto:`; boşluklar teke indirilir.
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        let lower = after.to_ascii_lowercase();
        let link = ["http://", "https://", "mailto:"]
            .iter()
            .any(|p| lower.starts_with(p));
        match after.find('>') {
            Some(j) if link => {
                out.push_str(&rest[..i]);
                rest = &after[j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace()
        .filter(|w| {
            let lower = w.to_ascii_lowercase();
            !["http://", "https://", "mailto:", "www."]
                .iter()
                .any(|p| lower.starts_with(p))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teams_block_is_dropped() {
        let raw = "Merhaba,\n\nLeaderboard önceliklendirmesi için davet.\n\n\
                   ________________________________________________________________________________\n\
                   Microsoft Teams Yardıma mı ihtiyacınız var?<https://aka.ms/JoinTeamsMeeting>\n\
                   Toplantıya şimdi katılın<https://teams.microsoft.com/l/meetup-join/x>\n";
        assert_eq!(
            agenda(raw),
            "Merhaba,\nLeaderboard önceliklendirmesi için davet."
        );
    }

    #[test]
    fn invitation_with_only_the_join_block_is_empty() {
        let raw = "\n________________\nMicrosoft Teams Need help?<https://aka.ms/x>\n\
                   Join the meeting now<https://teams.microsoft.com/l/x>\nMeeting ID: 123\n";
        assert_eq!(agenda(raw), "");
        assert_eq!(agenda("\n"), "");
    }

    #[test]
    fn html_is_turned_into_lines() {
        let raw = "<html><head><style>p{color:red}</style></head><body>\
                   <div>Gündem:</div><ul><li>Sprint &amp; demo</li><li>Riskler&nbsp;(LOY-214)</li></ul>\
                   <p>Detay: <a href=\"https://x.com/a\">https://x.com/a</a></p>\
                   <div>__________</div><div>Microsoft Teams meeting</div></body></html>";
        assert_eq!(
            agenda(raw),
            "Gündem:\nSprint & demo\nRiskler (LOY-214)\nDetay:"
        );
    }

    #[test]
    fn forwarded_header_and_links_are_dropped() {
        let raw = "Sunum taslağı ektedir. https://sharepoint.com/x <mailto:a@b.com>\n\
                   -----Original Appointment-----\nFrom: Ali\nSubject: Eski";
        assert_eq!(agenda(raw), "Sunum taslağı ektedir.");
    }

    #[test]
    fn forwarded_invitation_headers_are_skipped() {
        let raw = "From: a@b.com\nWhen: 13:00 - 14:00\nSubject: Araştırma\nLocation: Google Meet\n\
                   Merhaba Metin Bey,\nGörüşme akışı ektedir.";
        assert_eq!(agenda(raw), "Merhaba Metin Bey,\nGörüşme akışı ektedir.");
    }

    #[test]
    fn long_agendas_are_truncated() {
        let a = agenda(&"kelime ".repeat(200));
        assert_eq!(a.chars().count(), MAX_AGENDA_CHARS);
        assert!(a.ends_with('…'));
    }

    #[test]
    fn numeric_entities_and_stray_ampersands() {
        assert_eq!(
            decode_entities("A &#252; &#xE7; & B &bogus"),
            "A ü ç & B &bogus"
        );
    }
}
