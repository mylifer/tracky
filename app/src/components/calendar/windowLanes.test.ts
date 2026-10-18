import { describe, expect, it } from "vitest";
import type { WindowSpan } from "../../api";
import { placeItems, topWindow, windowItems } from "./windowLanes";

const T0 = Date.parse("2026-10-17T00:00:00+03:00");
const at = (hm: string) => {
  const [h, m] = hm.split(":").map(Number);
  return T0 + (h * 60 + m) * 60_000;
};
const iso = (hm: string) => new Date(at(hm)).toISOString();
const win = (from: string, to: string, appId: string, title: string, projectId: string | null = null): WindowSpan => ({
  start: iso(from),
  end: iso(to),
  appId,
  appName: appId,
  title,
  categoryId: null,
  projectId,
  domain: null,
});
const DAY_END = T0 + 86_400_000;

describe("windowItems", () => {
  it("aynı pencereye kısa sürede dönülünce tek öğe; geçen süre yalnızca o pencerede geçen", () => {
    const items = windowItems(
      [
        win("10:00", "10:03", "firefox", "Arama"),
        win("10:03", "10:05", "code", "api.ts"),
        win("10:05", "10:06", "firefox", "Arama"),
        // 20 dk sonra dönüş: yeni öğe.
        win("10:26", "10:28", "firefox", "Arama"),
      ],
      T0,
      DAY_END,
    );
    expect(items.map((i) => [i.title, i.start, i.end, i.ms, i.parts.length])).toEqual([
      ["Arama", at("10:00"), at("10:06"), 4 * 60_000, 2],
      ["api.ts", at("10:03"), at("10:05"), 2 * 60_000, 1],
      ["Arama", at("10:26"), at("10:28"), 2 * 60_000, 1],
    ]);
  });

  it("bir dakikadan kısa pencere çıkar; proje en çok sürenin", () => {
    const short = { ...win("09:00", "09:00", "x", "kısa"), end: new Date(at("09:00") + 50_000).toISOString() };
    const items = windowItems(
      [short, win("09:01", "09:05", "code", "api.ts", "p1"), win("09:05", "09:06", "code", "api.ts", null)],
      T0,
      DAY_END,
    );
    expect(items.map((i) => [i.title, i.projectId])).toEqual([["api.ts", "p1"]]);
  });
});

describe("placeItems", () => {
  const top = (t: number) => ((t - T0) / 3_600_000) * 60; // saatte 60 px

  it("çakışan öğe sonraki şeride kayar; sığmayan kısa öğe gizlenir", () => {
    const items = windowItems(
      [win("10:00", "11:00", "code", "uzun"), win("10:10", "10:15", "a", "orta"), win("10:12", "10:14", "b", "kısa")],
      T0,
      DAY_END,
    );
    const { placed, hidden } = placeItems(items, top, 2, 18);
    expect(placed.map((p) => [p.title, p.lane])).toEqual([
      ["uzun", 0],
      ["orta", 1],
    ]);
    expect(hidden).toBe(1);
    expect(placeItems(items, top, 3, 18).hidden).toBe(0);
  });
});

describe("topWindow", () => {
  // Uygulama olarak Firefox daha uzun sürer ama sekmelere dağılır; en uzun pencere Zoom'unki.
  const windows = [
    win("14:00", "14:20", "zoom", "Zoom Toplantısı"),
    win("14:20", "14:30", "firefox", "Arama"),
    win("14:30", "14:40", "firefox", "Haberler"),
    win("14:40", "14:55", "zoom", "Zoom Toplantısı"),
    win("14:55", "15:10", "firefox", "Pitch deck"),
    win("16:00", "18:00", "code", "api.ts"),
  ];

  it("bloğun adı aralıkta en çok süren pencere; aralığa kırpılır", () => {
    expect(topWindow(windows, at("14:00"), at("15:10"))).toEqual({
      appId: "zoom",
      appName: "zoom",
      title: "Zoom Toplantısı",
    });
    // Aralıktan önce başlayıp içine taşan uzun pencere de sayılır.
    expect(topWindow(windows, at("17:00"), at("17:30"))?.title).toBe("api.ts");
    expect(topWindow(windows, at("15:20"), at("15:50"))).toBeNull();
  });

  it("başlığı boş pencere uygulamanın adıyla", () => {
    expect(topWindow([win("09:00", "09:30", "Granola", " ")], at("09:00"), at("10:00"))?.title).toBe("Granola");
  });
});
