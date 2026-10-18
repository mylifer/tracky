import { expect, test as base, type Page } from "@playwright/test";
import type { Call } from "./backend";
import { NOW } from "./backend";

/** Sahte arka uçla açılmış uygulama: çağrılar okunur, olaylar gönderilir. */
export class App {
  constructor(readonly page: Page) {}

  /** Arka uca giden bu adlı çağrılar. */
  async calls(cmd: string): Promise<Call[]> {
    return this.page.evaluate((c) => window.__kum.calls.filter((x) => x.cmd === c), cmd);
  }

  /** Arka uçtan gelmiş gibi bir olay (ör. eşitleme bitti). */
  async emit(event: string, payload: unknown) {
    await this.page.evaluate(
      ([event, payload]) => window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event, payload }),
      [event, payload] as const,
    );
  }

  /** Kenar çubuğundaki sayfa düğmesi (rozetliyse adı "Gözden geçir 30dk" diye sürer). */
  nav(name: string) {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return this.page.getByRole("navigation").getByRole("button", { name: new RegExp(`^${escaped}( |,|$)`) });
  }

  /** Kenar çubuğundan sayfaya git. */
  async open(name: string) {
    await this.nav(name).click();
    await expect(this.nav(name)).toHaveAttribute("aria-current", "page");
  }
}

declare global {
  interface Window {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => Promise<unknown> };
  }
}

/**
 * Her test: saat sabit (`NOW`), sayfa hatası yok, çizim hatası ("Bu bölüm açılamadı") yok ve
 * sahte arka uçta tanımlı olmayan komut çağrılmadı.
 */
export const test = base.extend<{ app: App }>({
  app: async ({ page }, use) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.clock.setFixedTime(new Date(NOW));
    await page.goto("/e2e/harness.html");
    await expect(page.getByRole("navigation")).toBeVisible();
    await use(new App(page));
    const unmocked = await page.evaluate(() => [...new Set(window.__kum.unmocked)]);
    expect(unmocked, "sahte arka uçta tanımlı olmayan komutlar").toEqual([]);
    expect(errors, "sayfa hataları").toEqual([]);
    await expect(page.getByText("Bu bölüm açılamadı")).toHaveCount(0);
  },
});

export { expect };
