//! Arayüzün çekirdekten aldığı eşikler (`app/src/lib/core.ts`) Rust'taki karşılıklarıyla aynı mı?
//! Biri değişip öteki unutulursa takvim ile çizelge sessizce ayrışırdı.

use std::collections::BTreeMap;

use tracky_core::blocks;

/// `export const AD = 5 * 60_000;` satırları: ad ve değer (yalnızca sayıların çarpımı).
fn ui_constants() -> BTreeMap<String, i64> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/src/lib/core.ts");
    let text = std::fs::read_to_string(path).expect("app/src/lib/core.ts okunamadı");
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("export const "))
        .map(|rest| {
            let (name, expr) = rest.split_once('=').expect("sabit: ad = değer");
            let value = expr
                .trim()
                .trim_end_matches(';')
                .split('*')
                .map(|n| {
                    n.trim()
                        .replace('_', "")
                        .parse::<i64>()
                        .unwrap_or_else(|_| panic!("{name}: sayı değil: {n}"))
                })
                .product();
            (name.trim().to_string(), value)
        })
        .collect()
}

#[test]
fn ui_constants_match_the_core() {
    let core: BTreeMap<String, i64> = [("BREAK_GAP_MS", blocks::BREAK_GAP.num_milliseconds())]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    // Denetlenmeyen sabit kalmasın: core.ts'e eklenen her sabit burada da olmalı.
    assert_eq!(ui_constants(), core);
}
