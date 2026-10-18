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

/**
 * Sayfa durulsun: yarı saydam öğeler karşıtlıkta yanlış alarm verir. Sayfalar ilk açıldıklarında
 * yüklenir ve verisi sonra gelir; "şu an animasyon yok" yetmez, içerik taramanın ortasında
 * belirebilir (yavaş CI'da Gözden geçir kartı soluk yakalanıp sayı eşiği aşıyordu). Sayfa 300 ms
 * boyunca hem geçişsiz (sonsuz animasyonlar sayılmaz: canlı kartın nabzı, yükleme) hem değişmeden
 * kalınca durulmuş sayılır.
 */
const settle = async (page: Page) => {
  await page.evaluate(() => delete (window as { __settle?: unknown }).__settle);
  await page.waitForFunction(
    () => {
      const w = window as { __settle?: { size: number; since: number } };
      const moving = document
        .getAnimations()
        .some((a) => a.playState === "running" && a.effect?.getTiming().iterations !== Infinity);
      const size = document.body.innerHTML.length;
      const now = performance.now();
      if (moving || w.__settle?.size !== size) {
        w.__settle = { size, since: now };
        return false;
      }
      return now - w.__settle.since >= 300;
    },
    null,
    { timeout: 10_000, polling: 100 },
  );
};

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
