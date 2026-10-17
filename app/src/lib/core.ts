/**
 * Arayüzün çizimde (şeritler, çubuklar) kullandığı çekirdek eşikleri. Kural kararları (satır
 * sürüyor mu, bütçe eşiği, blok sınırları) çekirdekten hazır gelir; burada yalnızca çizimin
 * çekirdekle aynı sınırları göstermesi için gereken değerler durur. Her sabit Rust'taki
 * karşılığıyla aynı olmalı: `crates/tracky-core/tests/ui_constants.rs` bunu denetler, yeni sabit
 * eklenince oraya da eklenmeli.
 */

/** İki blok arasındaki mola sınırı (`blocks::BREAK_GAP`): bundan kısa boşluk işi bölmez. */
export const BREAK_GAP_MS = 5 * 60_000;
