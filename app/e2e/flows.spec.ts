import { PORTAL, SHEET } from "./backend";
import { expect, test } from "./fixtures";

test("Gözden geçir: grup önerilen projeye tek tuşla atanır", async ({ app, page }) => {
  await app.open("Gözden geçir");
  await expect(page.getByText("Hacker News")).toBeVisible();
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
  await expect(page.getByRole("button", { name: /^Zoom görüşmesi\s*45dk\s*11:35–12:20$/ })).toBeVisible();
  await page.getByRole("button", { name: /^portal — api\.ts\s*2sa\s*09:00–11:00$/ }).click();
  const menu = page.getByRole("dialog", { name: "Seçilen aralık" });
  await expect(menu).toContainText("09:00 – 11:00");
  await expect(menu).toContainText("Yalnızca Visual Studio Code · portal — api.ts");
});
