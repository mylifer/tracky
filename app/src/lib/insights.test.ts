import { describe, expect, it } from "vitest";
import { change, periodRanges, sumWeekly } from "./insights";
import { isoDate } from "./dates";

describe("dönem aralıkları", () => {
  // 6 Ekim 2026 Salı, 14:30.
  const now = new Date(2026, 9, 6, 14, 30);

  it("bu hafta, geçen haftanın aynı anıyla kıyaslanır", () => {
    const { cur, prev } = periodRanges("week", now);
    expect([isoDate(cur.start), cur.days]).toEqual(["2026-10-05", 7]);
    expect(isoDate(prev.start)).toBe("2026-09-28");
    expect(prev.until).toEqual(new Date(2026, 8, 29, 14, 30));
  });

  it("bu ay, geçen ayın aynı gününe kadar kıyaslanır", () => {
    const { cur, prev } = periodRanges("month", now);
    expect([isoDate(cur.start), cur.days]).toEqual(["2026-10-01", 31]);
    expect([isoDate(prev.start), prev.days]).toEqual(["2026-09-01", 30]);
    expect(prev.until).toEqual(new Date(2026, 8, 6, 14, 30));
  });

  it("ayın 31'i, kısa önceki ayın son gününe denk gelir", () => {
    const { prev } = periodRanges("month", new Date(2026, 2, 31, 9, 0));
    expect(prev.until).toEqual(new Date(2026, 1, 28, 9, 0));
  });

  it("son 3 ay bugünü de içeren 91 gün", () => {
    const { cur, prev } = periodRanges("quarter", now);
    expect(isoDate(cur.start)).toBe("2026-07-08");
    expect(isoDate(prev.start)).toBe("2026-04-08");
  });
});

describe("toplamlar", () => {
  it("önceki dönem boşsa değişim yok", () => {
    expect(change(15, 10)).toBe(0.5);
    expect(change(5, 0)).toBeNull();
  });

  it("haftalık serileri toplar, eksik proje sıfır sayılır", () => {
    const weekly = new Map([
      ["a", [1, 2, 3]],
      ["b", [10, 0, 5]],
    ]);
    expect(sumWeekly(weekly, ["a", "b", "yok"], 3)).toEqual([11, 2, 8]);
  });
});
