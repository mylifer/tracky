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
