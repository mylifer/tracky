import { describe, expect, it } from "vitest";
import { categoryBuckets, gapAround, HOUR_PX, placeSessions, subMarks } from "./Calendar";
import type { EntryView } from "../api";
import { sheetEntryAt, sheetSpans } from "./calendar/TimesheetLines";

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

describe("oturum yerleşimi", () => {
  const block = (a: number, b: number) =>
    ({
      ...span(a, b),
      activeSeconds: (b - a) / 1000,
      categoryId: null,
      projectId: null,
      switches: 0,
      topApps: [],
    }) as never;
  const top = (t: number) => ((t - at(8)) / (60 * MIN)) * HOUR_PX;

  it("kısa bloğu göstermez, kalanı en az çeyrek saat yüksekliğinde ama sonrakine binmeden çizer", () => {
    const placed = placeSessions(
      [block(at(9), at(9, 2)), block(at(10), at(10, 6)), block(at(10, 10), at(11)), block(at(12), at(12, 5))],
      [],
      top,
      HOUR_PX,
    );
    expect(placed.map((p) => p.start)).toEqual([at(10), at(10, 10), at(12)].map((t) => new Date(t).toISOString()));
    // 10:00 bloğu 10:10'dakine kadar uzar, 12:00 bloğu tam çeyrek saat.
    expect(placed[0].height).toBeCloseTo(top(at(10, 10)) - top(at(10)) - 2);
    expect(placed[2].height).toBeCloseTo(HOUR_PX / 4 - 2);
  });
});

describe("çizelge merceği", () => {
  const row = (date: string, start: string, hours: number, details: string): EntryView => ({
    date,
    start,
    hours,
    kind: "Working",
    details,
    party: "",
    projectId: "p1",
    division: "",
    id: null,
    key: details,
    exported: false,
    stale: null,
  });
  const spans = sheetSpans([
    row("2026-10-01", "09:00:00", 1, "Toplantı"),
    row("2026-10-01", "10:00:00", 2, "Geliştirme"),
    row("2026-10-02", "09:00:00", 1, "Ertesi gün"),
  ]);

  it("blokla en çok örtüşen satırı seçer", () => {
    expect(sheetEntryAt(spans, at(9, 40), at(11))?.details).toBe("Geliştirme");
    expect(sheetEntryAt(spans, at(9), at(9, 30))?.details).toBe("Toplantı");
  });

  it("satırın kendi tarihini kullanır; örtüşme yoksa satır yok", () => {
    expect(sheetEntryAt(spans, at(12, 30), at(13))).toBeUndefined();
    expect(sheetEntryAt(spans, at(33), at(33, 30))?.details).toBe("Ertesi gün");
  });

  it("bloğun ya da satırın yarısını kaplamayan örtüşmeyi saymaz", () => {
    // 3 saatlik blok, 09:00'daki satırın yalnızca ilk 5 dakikasına değiyor.
    expect(sheetEntryAt(spans, at(6, 5), at(9, 5))).toBeUndefined();
    // Uzun blok, içine tamamen giren satırları yine bulur.
    expect(sheetEntryAt(spans, at(8), at(12, 30))?.details).toBe("Geliştirme");
  });

  it("projesi olan blok yalnızca kendi projesinin satırını alır", () => {
    const other = sheetSpans([{ ...row("2026-10-01", "13:10:00", 0.25, "Başka iş"), projectId: "p2" }]);
    // 13:00–13:45 p1 bloğunun içine düşen kısa p2 satırı bloğun adı olmaz.
    expect(sheetEntryAt(other, at(13), at(13, 45), "p1")).toBeUndefined();
    expect(sheetEntryAt(other, at(13), at(13, 45), "p2")?.details).toBe("Başka iş");
    // Projesiz blok, kapsadığı satırın adını alır.
    expect(sheetEntryAt(other, at(13), at(13, 45))?.details).toBe("Başka iş");
  });

  it("satırın yazılan saatini değil kapsadığı takip aralıklarını kullanır", () => {
    // 07:00'den 3 saat yazılmış, ama iş 07:10–08:00 ve 09:30–10:30 arasında geçmiş.
    const tracked = sheetSpans([
      {
        ...row("2026-10-01", "07:00:00", 3, "Sabah"),
        coverage: [
          [at(7, 10), at(8)],
          [at(9, 30), at(10, 30)],
        ],
      },
    ]);
    // Yazılan aralığın içinde ama işin geçmediği 08:15–09:15 bloğu bu satırın değil.
    expect(sheetEntryAt(tracked, at(8, 15), at(9, 15))).toBeUndefined();
    // Yazılan aralığın dışında kalan ama satırın kapsadığı 10:00–10:30 bloğu bu satırın.
    expect(sheetEntryAt(tracked, at(10), at(10, 30))?.details).toBe("Sabah");
  });
});
