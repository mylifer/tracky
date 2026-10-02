//! Oturumları CSV olarak dışa aktarma (Excel/Numbers/Sheets ile açılabilir).

use chrono::{DateTime, Local, Utc};

use crate::classify::{Classifier, Tag};
use crate::model::Session;

const HEADER: &str = "baslangic,bitis,sure_sn,uygulama,uygulama_kimligi,baslik,kategori,proje";

/// Başında UTF-8 BOM olur; Excel Türkçe karakterleri böylece doğru açar.
pub fn sessions_csv(sessions: &[Session], tags: &[Tag], classifier: &Classifier) -> String {
    let name = |id: &Option<String>| {
        id.as_ref()
            .and_then(|id| tags.iter().find(|t| &t.id == id))
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    let mut out = String::from("\u{feff}");
    out.push_str(HEADER);
    out.push('\n');
    for s in sessions {
        let class = classifier.classify(s);
        let fields = [
            local(s.started_at),
            local(s.ended_at),
            s.duration().num_seconds().to_string(),
            s.app_name.clone(),
            s.app_id.clone(),
            s.title.clone(),
            name(&class.category),
            name(&class.project),
        ];
        let line: Vec<String> = fields.iter().map(|f| escape(f)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

fn local(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// RFC 4180: virgül, tırnak ya da satır sonu içeren alan tırnaklanır.
/// `=`, `+`, `-`, `@` ile başlayan alanlar tablo programlarında formül
/// sayılmasın diye başına `'` alır (CSV enjeksiyonu).
fn escape(field: &str) -> String {
    let field = if field.starts_with(['=', '+', '-', '@']) {
        format!("'{field}")
    } else {
        field.to_string()
    };
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    #[test]
    fn writes_escaped_rows() {
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let s = Session {
            id: Uuid::new_v4(),
            app_id: "com.apple.Safari".into(),
            app_name: "Safari".into(),
            title: "Bütçe, \"2026\" =SUM(A1)".into(),
            url: None,
            domain: None,
            started_at: t0,
            ended_at: t0 + chrono::Duration::seconds(90),
        };
        let csv = sessions_csv(&[s], &[], &Classifier::new(&[], &[]));
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap().trim_start_matches('\u{feff}'), HEADER);
        let row = lines.next().unwrap();
        assert!(row.contains(",90,Safari,com.apple.Safari,\"Bütçe, \"\"2026\"\" =SUM(A1)\",,"));
        assert_eq!(escape("=1+1"), "'=1+1");
    }
}
