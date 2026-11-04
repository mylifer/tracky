import { describe, expect, it } from "vitest";
import type { WindowSpan } from "../api";
import { assignRanges, draftSlots, mergeWith, normalize, paint, resize, type Slot, split } from "./plan";

const DAY = Date.parse("2026-11-03T00:00:00");
const at = (hm: string) => {
  const [h, m] = hm.split(":").map(Number);
  return DAY + (h * 60 + m) * 60_000;
};
const hm = (t: number) => new Date(t).toTimeString().slice(0, 5);
const NOW = at("14:40");

function win(from: string, to: string, projectId: string | null, appName = "App"): WindowSpan {
  return {
    start: new Date(at(from)).toISOString(),
    end: new Date(at(to)).toISOString(),
    appId: appName,
    appName,
    title: appName,
    categoryId: null,
    projectId,
    domain: null,
  };
}

const show = (list: Slot[]) => list.map((b) => `${hm(b.start)}–${hm(b.end)} ${b.projectId ?? "-"}`);

// Ekran görüntüsündeki günün kabası: gece oyun, sabah yayın (arada 11 dk kalkış), SD App,
// arada 44 dk Kum, öğle arası, öğleden sonra Kum.
const DAY_WINDOWS = [
  win("02:28", "02:44", null, "Discord"),
  win("02:50", "03:17", "oyun"),
  win("03:17", "03:23", "oyun"),
  win("03:23", "03:27", null, "Önizleme"),
  win("03:30", "04:01", "oyun"),
  win("04:01", "04:12", null, "Claude"),
  win("04:12", "04:16", "kum"),
  win("04:16", "04:21", "togg"),
  win("04:22", "05:52", "yayin"),
  win("06:03", "06:50", "yayin"),
  win("06:50", "06:57", null, "Claude"),
  win("06:57", "07:01", "togg"),
  win("07:01", "08:40", "yayin"),
  win("09:00", "09:54", "togg"),
  win("10:00", "10:04", null, "WhatsApp"),
  win("10:13", "10:36", "kum"),
  win("10:36", "10:44", "togg"),
  win("10:44", "10:57", "kum"),
  win("10:57", "11:06", null, "Claude"),
  win("11:06", "12:09", "togg"),
  win("14:23", "14:40", "kum"),
];

describe("taslak", () => {
  it("günü birkaç büyük bloğa indirir; kısa kalkış ve araya girmeler blok açmaz", () => {
    const drafts = draftSlots(DAY_WINDOWS, DAY, DAY + 86_400_000, NOW, []);
    expect(show(drafts)).toEqual([
      "02:30–04:15 oyun",
      "04:15–08:40 yayin",
      "09:00–10:15 togg",
      "10:15–11:00 kum",
      "11:00–12:10 togg",
      "14:20–14:40 kum",
    ]);
    expect(drafts.every((b) => b.draft)).toBe(true);
  });

  it("onaylı blokların aralığına girmez", () => {
    const confirmed: Slot[] = [{ id: "a", start: at("09:00"), end: at("12:10"), projectId: "togg", note: "" }];
    const drafts = draftSlots(DAY_WINDOWS, DAY, DAY + 86_400_000, NOW, confirmed);
    expect(drafts.some((b) => b.end > at("09:00") && b.start < at("12:10"))).toBe(false);
  });

  it("atanmamış süre yandaki projeli bloğu uzatmaz", () => {
    // 09:00–09:30 Trumore, ardından 45 dk atanmamış tarayıcı, sonra yine Trumore.
    const w = [win("09:00", "09:30", "tru"), win("09:30", "10:15", null, "Safari"), win("10:15", "11:00", "tru")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual([
      "09:00–09:30 tru",
      "09:30–10:15 -",
      "10:15–11:00 tru",
    ]);
  });

  it("şu an süren atanmamış iş projeli bloğa katılmaz (blok dakika dakika büyümez)", () => {
    // 14:00–14:20 Trumore, 14:20'den beri (şimdi 14:31) atanmamış.
    const w = [win("13:00", "14:20", "tru"), win("14:20", "14:31", null, "Safari")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, at("14:31"), []))).toEqual(["13:00–14:15 tru", "14:15–14:30 -"]);
  });

  it("bir dilimlik atanmamış araya girme önceki blokta kalır", () => {
    const w = [win("09:00", "10:00", "tru"), win("10:00", "10:12", null, "WhatsApp"), win("10:15", "11:00", "kum")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual(["09:00–10:15 tru", "10:15–11:00 kum"]);
  });

  it("şimdiden sonrasını çizmez", () => {
    const drafts = draftSlots([win("14:00", "15:00", "togg")], DAY, DAY + 86_400_000, at("14:30"), []);
    expect(show(drafts)).toEqual(["14:00–14:30 togg"]);
  });
});

const b = (id: string, from: string, to: string, projectId: string | null = "p"): Slot => ({
  id,
  start: at(from),
  end: at(to),
  projectId,
  note: "",
});

describe("düzenleme", () => {
  it("kenar sürükleme yalnızca kenarı taşır; bitişik komşu da kayar, yeni blok doğmaz", () => {
    const list = [b("a", "09:00", "10:00"), b("b", "10:00", "11:00", "q")];
    const out = resize(list, "a", "end", at("10:20"), NOW);
    expect(show(out)).toEqual(["09:00–10:20 p", "10:20–11:00 q"]);
    // Küçültmek de parça doğurmaz.
    expect(show(resize(list, "a", "end", at("09:40"), NOW))).toEqual(["09:00–09:40 p", "09:40–11:00 q"]);
    // Komşu 5 dakikadan kısalamaz.
    expect(show(resize(list, "a", "end", at("11:30"), NOW))).toEqual(["09:00–10:55 p", "10:55–11:00 q"]);
  });

  it("bitişik olmayan komşunun üstüne binmez, şimdiyi geçmez", () => {
    const list = [b("a", "09:00", "10:00"), b("b", "10:30", "11:00", "q")];
    expect(show(resize(list, "a", "end", at("10:45"), NOW))).toEqual(["09:00–10:30 p", "10:30–11:00 q"]);
    expect(show(resize(list, "b", "end", at("16:00"), NOW))).toEqual(["09:00–10:00 p", "10:30–14:40 q"]);
  });

  it("boyama aralığı tek blok yapar, kesişenleri kırpar ya da böler", () => {
    const list = [b("a", "09:00", "12:00"), b("c", "12:00", "13:00", "q")];
    const out = paint(list, at("10:00"), at("12:30"), "x");
    expect(show(out)).toEqual(["09:00–10:00 p", "10:00–12:30 x", "12:30–13:00 q"]);
    const inside = paint([b("a", "09:00", "12:00")], at("10:00"), at("11:00"), "x");
    expect(show(inside)).toEqual(["09:00–10:00 p", "10:00–11:00 x", "11:00–12:00 p"]);
    expect(new Set(inside.map((x) => x.id)).size).toBe(3);
  });

  it("aynı projeye boyanan bitişik bloklar birleşir", () => {
    const out = paint([b("a", "09:00", "10:00"), b("c", "10:00", "11:00", "q")], at("10:00"), at("11:00"), "p");
    expect(show(out)).toEqual(["09:00–11:00 p"]);
  });

  it("bölme iki parça verir, birleştirme aradaki boşluğu kapatır", () => {
    const parts = split([b("a", "09:00", "11:00")], "a", at("10:00"));
    expect(show(parts)).toEqual(["09:00–10:00 p", "10:00–11:00 p"]);
    expect(split([b("a", "09:00", "11:00")], "a", at("09:02"))).toHaveLength(1);
    const merged = mergeWith([b("a", "09:00", "09:30", "x"), b("c", "09:40", "11:00")], "a", "next");
    expect(show(merged)).toEqual(["09:00–11:00 p"]);
  });

  it("normalize kısa parçaları atar, aynı projeli bitişikleri birleştirir", () => {
    const out = normalize([b("a", "09:00", "10:00"), b("c", "10:00", "10:00"), b("d", "10:00", "11:00")]);
    expect(show(out)).toEqual(["09:00–11:00 p"]);
  });

  it("yalnızca değişen ve projesi belli kısımlar oturumlara yazılır", () => {
    const before = [b("a", "09:00", "10:00", "p")];
    const after = [b("a", "09:00", "10:30", "p"), b("c", "10:30", "11:00", null)];
    expect(assignRanges(before, after).map((r) => `${hm(r.start)}–${hm(r.end)} ${r.projectId}`)).toEqual([
      "10:00–10:30 p",
    ]);
  });
});
