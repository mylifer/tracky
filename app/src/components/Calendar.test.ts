import { describe, expect, it } from "vitest";
import { gapAround, smallLabels } from "./Calendar";

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

describe("kısa çubuk etiketleri", () => {
  // 1 dakika = 1 piksel; çubuk yüksekliği blockGeometry'de 2 piksel kısalır.
  const range = { first: 0, last: 24, px: 60 };
  const top = (t: number) => (t - day) / MIN;
  const app = (name: string, a: number, b: number) => ({ ...span(day + a * MIN, day + b * MIN), appName: name });

  it("içine sığmayanlara etiket koyar, çarpışınca kaydırır, çok kayacaksa atlar", () => {
    const labels = smallLabels(
      [app("Uzun", 0, 30), app("A", 30, 34), app("B", 34, 38), app("C", 38, 42), app("D", 42, 46), app("E", 46, 50)],
      top,
      range,
    );
    // C ve D çok kayacağı için atlanır; E'ye gelince yer açılmıştır.
    expect(labels.map((l) => l.name)).toEqual(["A", "B", "E"]);
    // Her etiket bir öncekinin altında, üst üste binmeden.
    labels.slice(1).forEach((l, i) => expect(l.y).toBeGreaterThanOrEqual(labels[i].y + 11));
  });

  it("bir sonraki adlı çubuğun yazısını örtmez", () => {
    const labels = smallLabels([app("A", 0, 4), app("Uzun", 6, 40)], top, range);
    expect(labels).toEqual([]);
  });
});
