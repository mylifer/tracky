import { describe, expect, it } from "vitest";
import { timeTicks } from "./AppTimeline";

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
