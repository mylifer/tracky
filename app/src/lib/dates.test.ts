import { describe, expect, it } from "vitest";
import { addDays, fromWallMs, isoDate, parseIsoDate, startOfWeek, wallMs } from "./dates";

const H = 3600_000;

describe("duvar saati (Europe/Berlin)", () => {
  // 29 Mart 2026: 02:00 → 03:00 (gün 23 saat); 25 Ekim 2026: 03:00 → 02:00 (gün 25 saat).
  it.each([
    ["yaz saatine geçiş", new Date(2026, 2, 29)],
    ["kış saatine geçiş", new Date(2026, 9, 25)],
    ["sıradan gün", new Date(2026, 5, 1)],
  ])("%s: saat etiketleriyle hizalı ve tersinir", (_, day) => {
    const ds = +day;
    for (const h of [1, 4, 9, 15, 22]) {
      const t = +new Date(day.getFullYear(), day.getMonth(), day.getDate(), h, 30);
      expect(wallMs(t, ds) / H).toBe(h + 0.5);
      expect(fromWallMs((h + 0.5) * H, ds)).toBe(t);
    }
  });

  it("geçiş gününde geçen süre duvar saatinden farklıdır", () => {
    const ds = +new Date(2026, 2, 29);
    const t = +new Date(2026, 2, 29, 4, 0);
    expect((t - ds) / H).toBe(3);
    expect(wallMs(t, ds) / H).toBe(4);
  });
});

describe("tarih yardımcıları", () => {
  it("ISO tarih gidiş-dönüş", () => {
    expect(isoDate(parseIsoDate("2026-10-03"))).toBe("2026-10-03");
  });

  it("gün ekleme yaz saatinde de gece yarısında kalır", () => {
    const d = addDays(new Date(2026, 2, 28), 1);
    expect([d.getDate(), d.getHours()]).toEqual([29, 0]);
  });

  it("haftanın pazartesisi", () => {
    expect(isoDate(startOfWeek(new Date(2026, 9, 4)))).toBe("2026-09-28");
    expect(isoDate(startOfWeek(new Date(2026, 8, 28)))).toBe("2026-09-28");
  });
});
