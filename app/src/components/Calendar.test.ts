import { describe, expect, it } from "vitest";
import { categoryBuckets, gapAround, HOUR_PX, subMarks } from "./Calendar";

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

describe("kategori şeridi", () => {
  const seg = (a: number, b: number, categoryId: string | null) => ({
    ...span(a, b),
    appName: "x",
    title: "",
    categoryId,
  });

  it("her aralığa en çok süren kategoriyi verir, boş aralığı atlar", () => {
    const buckets = categoryBuckets(
      [seg(at(9), at(9, 10), "dev"), seg(at(9, 10), at(9, 15), null), seg(at(9, 20), at(9, 25), null)],
      day,
      15,
    );
    expect(buckets.map((b) => [b.start, b.categoryId])).toEqual([
      [at(9), "dev"],
      [at(9, 15), null],
    ]);
    expect(buckets[0].coverage).toBe(1);
    expect(buckets[0].shares.map((c) => c.id)).toEqual(["dev", null]);
    expect(buckets[1].coverage).toBeCloseTo(1 / 3);
  });

  it("çok kısa takip edilen aralığı göstermez", () => {
    expect(categoryBuckets([seg(at(10), at(10) + 10_000, "dev")], day, 15)).toEqual([]);
  });
});

describe("subMarks", () => {
  it("varsayılan görünümde yarım saat, yakınlaşınca çeyrek, çok uzakta yok", () => {
    expect(subMarks(HOUR_PX, false)).toEqual([30]);
    expect(subMarks(HOUR_PX, true)).toEqual([30]);
    expect(subMarks(240, false)).toEqual([15, 30, 45]);
    expect(subMarks(320, true)).toEqual([15, 30, 45]);
    expect(subMarks(20, false)).toEqual([]);
    expect(subMarks(30, true)).toEqual([]);
  });
});
