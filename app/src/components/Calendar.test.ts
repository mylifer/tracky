import { describe, expect, it } from "vitest";
import { gapAround } from "./Calendar";

const MIN = 60_000;
const day = +new Date(2026, 9, 1);
const at = (h: number, m = 0) => day + (h * 60 + m) * MIN;
const span = (a: number, b: number) => ({ start: new Date(a).toISOString(), end: new Date(b).toISOString() });

describe("boş alana tıklama", () => {
  const spans = [span(at(9), at(10)), span(at(11), at(11, 30)), span(at(16), at(17))];

  it("2 saate kadar boşluğun tamamını seçer", () => {
    expect(gapAround(at(10, 20), spans, day)).toEqual([at(10), at(11)]);
  });

  it("uzun boşlukta tıklanan çeyrekten 1 saat", () => {
    expect(gapAround(at(13, 40), spans, day)).toEqual([at(13, 30), at(14, 30)]);
  });

  it("dolu alanda ya da çok kısa boşlukta seçmez", () => {
    expect(gapAround(at(9, 30), spans, day)).toBeNull();
    expect(gapAround(at(10, 58), [span(at(9), at(10, 57)), span(at(11), at(12))], day)).toBeNull();
  });
});
