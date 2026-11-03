import { PORTAL, SHEET } from "./backend";
import { expect, test } from "./fixtures";

test("Gözden geçir: grup önerilen projeye tek tuşla atanır", async ({ app, page }) => {
  await app.open("Gözden geçir");
  // Kartın pencere satırı (geçişte önceki sayfanın takviminde de aynı ad görünebilir).
  await expect(page.getByRole("listitem").filter({ hasText: "Hacker News" })).toBeVisible();
  // 1: ilk seçenek, öncesinde/sonrasında çalışılan proje (Portal).
  await page.keyboard.press("1");
  await expect.poll(async () => (await app.calls("assign_unassigned")).length).toBe(1);
  const [call] = await app.calls("assign_unassigned");
  expect(call.args).toMatchObject({ key: "app:com.apple.Safari", projectId: PORTAL, title: null });
});

test("Ayarlar: eski sürümde kalan bilgisayar uyarılır", async ({ app, page }) => {
  await app.open("Ayarlar");
  await expect(page.getByRole("status").filter({ hasText: "MacBook Air eski sürümde" })).toBeVisible();
  await expect(page.getByText("Sürüm 0.9.70")).toBeVisible();
  await expect(page.getByText(/Düzenlemeler öteki bilgisayarlara birkaç saniyede gider/)).toBeVisible();
  await expect(page.getByText("Sürüm 0.9.74")).toBeVisible();
});

test("Zaman çizelgesi: çekirdeğe göre süren satır toplu gönderime girmez", async ({ app, page }) => {
  await app.open("Zaman çizelgesi");
  // 14:00: öğleden sonraki satırın işi 14:05'e kadar sürüyor (son işinden sonra 15 dk dolmadı).
  const waiting = (title: string) => page.getByTitle(title).filter({ hasText: "bekliyor", visible: true });
  await expect(waiting("Başlamamış ya da süren 1 satır hariç")).toContainText("1");
  expect((await app.calls("timesheet_days")).at(-1)?.args).toMatchObject({ timesheetId: SHEET });
  // 14:10: iş bitti, ikisi de gönderilebilir.
  await page.clock.setFixedTime(new Date("2026-10-07T14:10:00+03:00"));
  await app.emit("sync", null);
  await expect(waiting("Gönderilmemiş satırlar")).toContainText("2");
});

test("Yenilikler: kurulan sürümün notları bir kez gösterilir, kapatılır", async ({ app, page }) => {
  const card = page.getByRole("region", { name: "Kum 0.9.75 yenilikleri" });
  await expect(card).toContainText("Güncellemeler bilgisayar başında değilken kendiliğinden kurulur");
  await card.getByRole("button", { name: "Yenilikleri kapat" }).click();
  await expect(card).toHaveCount(0);
  expect(await app.calls("dismiss_whats_new")).toHaveLength(1);
});

test("Ayarlar: güncellemeleri kendiliğinden kurma kapatılabilir", async ({ app, page }) => {
  await app.open("Ayarlar");
  const toggle = page.getByRole("switch", { name: "Güncellemeleri kendiliğinden kur" });
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  expect((await app.calls("set_auto_update"))[0].args).toEqual({ enabled: false });
});

test("Ayarlar: veri denetimi bulguları gösterir ve onarır", async ({ app, page }) => {
  await app.open("Ayarlar");
  await page.getByRole("button", { name: "Denetle", exact: true }).click();
  await expect(page.getByText("2 bulgu, 2 tanesi onarılabilir.")).toBeVisible();
  await expect(page.getByText(/firefox görüşmesi/)).toBeVisible();
  await page.getByRole("button", { name: "Onar (2)", exact: true }).click();
  await expect(page.getByText("Sorun bulunmadı.")).toBeVisible();
  expect(await app.calls("repair_data")).toHaveLength(1);
});

test("Gün: Pencereler sütunu her pencereyi saatinde gösterir, tıklanan yalnızca o pencereyi atar", async ({
  app,
  page,
}) => {
  await app.open("Gün");
  await page.getByRole("button", { name: /^portal — api\.ts\s*2sa\s*09:00–11:00$/ }).click();
  const menu = page.getByRole("dialog", { name: "Seçilen aralık" });
  await expect(menu).toContainText("09:00 – 11:00");
  await expect(menu).toContainText("Yalnızca Visual Studio Code · portal — api.ts");
});

test("Gün: taslak kısa araya girmeyi tek kutuda gösterir, parçalar açılabilir", async ({ app, page }) => {
  await app.open("Gün");
  // 09:00–11:00 portal ve ardından 30 dk projesiz Safari: 15 dk eşikte tek taslak.
  const draft = page.locator("[data-draft]");
  await expect(draft).toHaveCount(1);
  await expect(draft).toContainText("2 parça");
  await expect(page.getByRole("button", { name: "Hacker News 30dk", exact: true })).toHaveCount(0);
  await draft.click();
  await expect(page.getByRole("dialog")).toContainText("09:00 – 11:30");
  await page.getByRole("button", { name: "Parçaları aç" }).click();
  await expect(draft).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Hacker News 30dk", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Topla" }).click();
  await expect(draft).toHaveCount(1);
});

test("Gün: blok kenarı uzun uygulama kullanımının sınırına yapışır, ⌥ basılıyken yapışmaz", async ({ app, page }) => {
  await app.open("Gün");
  // Kenar sürükleme tek tek bloklarda: taslak kapatılır.
  await page.getByRole("button", { name: "Taslak eşiği: 15 dk" }).click();
  await page.getByRole("button", { name: "Ayrıntılı: her blok ayrı" }).click();
  // Menü kapanırken sürükleme onun üstünde başlamasın.
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // Safari 11:00–11:30; Visual Studio Code 12:30'da yeniden başlıyor.
  const block = page.getByRole("button", { name: "Hacker News 30dk", exact: true });
  const box = (await block.boundingBox())!;
  const hourPx = (box.height + 2) * 2;
  const x = box.x + box.width / 2;
  await page.mouse.move(x, box.y + box.height - 2);
  await page.mouse.down();
  // 12:26'ya çekilen bitiş dilime (12:25) değil, kümenin başına (12:30) oturur.
  const y = box.y + box.height - 2 + (56 / 60) * hourPx;
  await page.mouse.move(x, y, { steps: 6 });
  await expect(page.getByText("11:00 – 12:30")).toBeVisible();
  await expect(page.locator("[data-snap=app]")).toHaveText("Visual Studio Code · başı");
  await page.keyboard.down("Alt");
  await page.mouse.move(x, y + 1);
  await expect(page.getByText("11:00 – 12:25")).toBeVisible();
  await expect(page.locator("[data-snap]")).toHaveCount(0);
  await page.keyboard.up("Alt");
  await page.mouse.move(x, y);
  await expect(page.getByText("11:00 – 12:30")).toBeVisible();
  await page.mouse.up();
  const [call] = await app.calls("resize_block");
  expect(new Date(call.args.newEnd as string).toISOString()).toBe(new Date("2026-10-07T12:30:00+03:00").toISOString());
});

test("Zaman çizelgesi: ayı kapatırken takip edilen süreden dosyaya döküm gösterilir", async ({ app, page }) => {
  await app.open("Zaman çizelgesi");
  await page.getByRole("tablist", { name: "Görünüm" }).getByRole("tab", { name: "Ay" }).click();
  await page.getByRole("button", { name: "Ayı kapat" }).click();
  const panel = page.getByRole("region", { name: "Ayı kapat: denetim" });
  await expect(panel.getByText("Takip edilen süreden Excel'e")).toBeVisible();
  await expect(panel.getByText("Satırların gerçek süresi")).toBeVisible();
  // Konular tek satırlık özettir; tıklayınca günler açılır.
  const unassigned = panel.getByRole("button", { name: /Projeye atanmamış süre/ });
  await expect(unassigned).toHaveAttribute("aria-expanded", "false");
  await unassigned.click();
  await expect(panel.getByRole("button", { name: "gözden geçir" })).toBeVisible();
  if (process.env.SHOT) await panel.screenshot({ path: process.env.SHOT });
});
