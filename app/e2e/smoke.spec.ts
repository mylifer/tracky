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

test("her sayfa açılır, hiçbiri çizim hatası vermez", async ({ app, page }) => {
  for (const name of PAGES) {
    await app.open(name);
    await expect(page.getByRole("alert").filter({ hasText: "Bu bölüm açılamadı" })).toHaveCount(0);
  }
});
