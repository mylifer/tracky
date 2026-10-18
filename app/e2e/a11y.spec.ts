import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

const PAGES = [
  "Bugün",
  "Gözden geçir",
  "Zaman çizelgesi",
  "Gün",
  "Hafta",
  "Ay",
  "Eğilimler",
  "Müşteri raporu",
  "Ara",
  "Müşteriler",
  "Projeler",
  "Kategoriler",
  "Ayarlar",
];

/**
 * Karşıtlığı WCAG AA'nın altında kalan öğe sayısı (bütün sayfalar; 2026-10-18'de Chromium 54,
 * WebKit 55). Çoğu küçük ikincil yazı (`text-muted-foreground`, 11 px ve altı: çizelge sütun
 * başlıkları, takvimin ":30" çizgileri, ay takviminin gün numaraları) ve macOS sistem mavisi
 * düğmelerde beyaz yazı (4,0:1). Bu sayı artmasın; tasarım düzeltildikçe düşürülür.
 */
const CONTRAST_BASELINE = 55;

/** Sayfa geçiş animasyonu bitsin: yarı saydam öğeler karşıtlıkta yanlış alarm verir. */
const settle = (page: Page) =>
  page.waitForFunction(
    () =>
      document
        .getAnimations()
        // Sonsuz animasyonlar (canlı kartın nabzı, yükleme) hiç bitmez: yalnızca geçişler beklenir.
        .every((a) => a.playState !== "running" || a.effect?.getTiming().iterations === Infinity),
    null,
    {
      timeout: 3000,
    },
  );

const scan = (page: Page) => new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]);

for (const name of PAGES) {
  test(`erişilebilirlik: ${name}`, async ({ app, page }) => {
    await app.open(name);
    await settle(page);
    const result = await scan(page).disableRules(["color-contrast"]).analyze();
    const found = result.violations.map(
      (v) =>
        `${v.impact} ${v.id}: ${v.nodes
          .slice(0, 3)
          .map((n) => n.html.slice(0, 120))
          .join(" | ")}`,
    );
    expect(found, "erişilebilirlik ihlalleri").toEqual([]);
  });
}

test("erişilebilirlik: karşıtlık sorunları artmaz", async ({ app, page }) => {
  let nodes = 0;
  const where: string[] = [];
  for (const name of PAGES) {
    await app.open(name);
    await settle(page);
    const result = await scan(page).withRules(["color-contrast"]).analyze();
    for (const v of result.violations) {
      nodes += v.nodes.length;
      where.push(`${name}: ${v.nodes.map((n) => n.html.slice(0, 80)).join(" | ")}`);
    }
  }
  expect(nodes, where.join("\n")).toBeLessThanOrEqual(CONTRAST_BASELINE);
});
