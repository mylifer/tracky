import { describe, expect, it } from "vitest";
import type { CallSpan, WindowSpan } from "../../api";
import { placeItems, windowItems } from "./windowLanes";

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
const call = (from: string, to: string, appId: string, browser = false): CallSpan => ({
  start: iso(from),
  end: iso(to),
  appId,
  appName: appId,
  browser,
});
const DAY_END = T0 + 86_400_000;

describe("windowItems", () => {
  it("aynı pencereye kısa sürede dönülünce tek öğe; geçen süre yalnızca o pencerede geçen", () => {
    const items = windowItems(
      [
        win("10:00", "10:03", "firefox", "Arama"),
        win("10:03", "10:05", "code", "api.ts"),
        win("10:05", "10:06", "firefox", "Arama"),
        // 10 dk sonra dönüş: yeni öğe.
        win("10:16", "10:18", "firefox", "Arama"),
      ],
      [],
      T0,
      DAY_END,
    );
    expect(items.map((i) => [i.title, i.start, i.end, i.ms, i.parts.length])).toEqual([
      ["Arama", at("10:00"), at("10:06"), 4 * 60_000, 2],
      ["api.ts", at("10:03"), at("10:05"), 2 * 60_000, 1],
      ["Arama", at("10:16"), at("10:18"), 2 * 60_000, 1],
    ]);
  });

  it("bir dakikadan kısa pencere çıkar; proje en çok sürenin", () => {
    const short = { ...win("09:00", "09:00", "x", "kısa"), end: new Date(at("09:00") + 50_000).toISOString() };
    const items = windowItems(
      [short, win("09:01", "09:05", "code", "api.ts", "p1"), win("09:05", "09:06", "code", "api.ts", null)],
      [],
      T0,
      DAY_END,
    );
    expect(items.map((i) => [i.title, i.projectId])).toEqual([["api.ts", "p1"]]);
  });

  it("görüşme uygulamasının görüşme sırasındaki pencereleri görüşmeye katılır; tarayıcıda kalır", () => {
    const items = windowItems(
      [
        win("13:55", "14:30", "zoom", "Zoom Toplantısı"),
        win("14:30", "14:40", "firefox", "Meet"),
        win("14:40", "14:50", "zoom", "Zoom Toplantısı"),
      ],
      [call("14:00", "14:45", "zoom"), call("14:30", "14:40", "firefox", true)],
      T0,
      DAY_END,
    );
    expect(items.map((i) => [i.kind, i.appId, i.start, i.end, i.ms])).toEqual([
      ["window", "zoom", at("13:55"), at("14:00"), 5 * 60_000],
      ["call", "zoom", at("14:00"), at("14:45"), 45 * 60_000],
      ["call", "firefox", at("14:30"), at("14:40"), 10 * 60_000],
      ["window", "firefox", at("14:30"), at("14:40"), 10 * 60_000],
      ["window", "zoom", at("14:45"), at("14:50"), 5 * 60_000],
    ]);
  });
});

describe("placeItems", () => {
  const top = (t: number) => ((t - T0) / 3_600_000) * 60; // saatte 60 px

  it("çakışan öğe sonraki şeride kayar; sığmayan kısa öğe gizlenir", () => {
    const items = windowItems(
      [win("10:00", "11:00", "code", "uzun"), win("10:10", "10:15", "a", "orta"), win("10:12", "10:14", "b", "kısa")],
      [],
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

  it("görüşmenin çubuğu rayda: şeritte yalnızca etiketi yer kaplar", () => {
    const items = windowItems([win("10:30", "10:40", "code", "api.ts")], [call("10:00", "12:00", "zoom")], T0, DAY_END);
    const { placed } = placeItems(items, top, 1, 18);
    expect(placed.map((p) => [p.kind, p.lane, p.height])).toEqual([
      ["call", 0, 120],
      ["window", 0, 18],
    ]);
  });
});
