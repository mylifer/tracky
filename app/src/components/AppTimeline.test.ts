import { describe, expect, it } from "vitest";
import { slotBars, slotMinutes, timeTicks } from "./AppTimeline";

const MIN = 60_000;
const H = 60 * MIN;

describe("çizelge zaman işaretleri", () => {
  it("tüm gün: saat başları", () => {
    const t = timeTicks(8 * H, 12 * H);
    expect(t).toHaveLength(13);
    expect(t.every((x) => x % H === 0)).toBe(true);
  });

  it("yakınlaşınca en çok 12 çizgi veren en sık adım", () => {
    const two = timeTicks(12 * H + 5 * MIN, 2 * H);
    expect(two[0]).toBe(12 * H + 10 * MIN);
    expect(two[1] - two[0]).toBe(10 * MIN);
    const twoHalf = timeTicks(12 * H + 5 * MIN, 2.5 * H);
    expect(twoHalf[0]).toBe(12 * H + 15 * MIN);
    expect(twoHalf[1] - twoHalf[0]).toBe(15 * MIN);
  });

  it("çok yakında 5 dakika", () => {
    const t = timeTicks(9 * H, 30 * MIN);
    expect(t[1] - t[0]).toBe(5 * MIN);
  });
});

describe("çizelge dilimleri", () => {
  const day = +new Date(2026, 9, 1);
  const starts = [day, day + 24 * H];
  const at = (h: number, m = 0) => new Date(day + h * H + m * MIN).toISOString();
  const w = (a: string, b: string, title = "t", categoryId: string | null = "dev") => ({
    start: a,
    end: b,
    appId: "a",
    appName: "A",
    title,
    categoryId,
  });

  it("tam günde 15 dk, yakınlaşınca incelir, haftada büyür", () => {
    expect(slotMinutes(1, 1)).toBe(15);
    expect(slotMinutes(1, 1.5)).toBe(10);
    expect(slotMinutes(1, 3)).toBe(5);
    expect(slotMinutes(7, 1)).toBe(60);
    expect(slotMinutes(7, 8)).toBe(15);
  });

  it("üçte biri dolan dilim dolu, art arda dilimler tek çubuk; az dolan boş", () => {
    const bars = slotBars(
      [
        w(at(9, 2), at(9, 14), "kod"),
        w(at(9, 15), at(9, 21), "pr"),
        w(at(9, 31), at(9, 32)), // 1 dk: boş kalır
        w(at(10), at(10, 1)),
        w(at(10, 1), at(10, 5)), // toplam 5 dk: dolu
      ] as never,
      starts,
      15,
    );
    expect(bars.map((b) => [b.start, b.end])).toEqual([
      [day + 9 * H, day + 9 * H + 30 * MIN],
      [day + 10 * H, day + 10 * H + 15 * MIN],
    ]);
    expect(bars[0].ms).toBe(18 * MIN);
    expect(bars[0].titles.map((t) => t.title)).toEqual(["kod", "pr"]);
  });
});
