import { describe, expect, it } from "vitest";
import type { WindowSpan } from "../api";
import { blockWindows, projectShare } from "./blockWindows";

const at = (min: number) => new Date(Date.UTC(2026, 9, 5, 9, min)).toISOString();
const span = (app: string, title: string, from: number, to: number, projectId: string | null = null): WindowSpan => ({
  start: at(from),
  end: at(to),
  appId: `com.${app}`,
  appName: app,
  title,
  categoryId: null,
  projectId,
  domain: app === "Chrome" ? "jira.firma.com" : null,
});

describe("takvim bloğundaki pencereler", () => {
  it("uygulamaya göre gruplar, aynı başlığı toplar, uzundan kısaya sıralar", () => {
    const apps = blockWindows(
      [
        span("Code", "sync.rs — kum", 0, 10),
        span("Chrome", "LOY-214 · Jira", 10, 15),
        span("Code", "lib.rs — kum", 15, 20),
        span("Code", "sync.rs — kum", 20, 30),
      ],
      at(0),
      at(30),
    );
    expect(apps.map((a) => [a.appName, a.seconds])).toEqual([
      ["Code", 25 * 60],
      ["Chrome", 5 * 60],
    ]);
    expect(apps[0].windows.map((w) => [w.title, w.seconds])).toEqual([
      ["sync.rs — kum", 20 * 60],
      ["lib.rs — kum", 5 * 60],
    ]);
    expect(apps[1].windows[0].domain).toBe("jira.firma.com");
  });

  it("bloğun dışındaki kısmı saymaz, farklı projeye atanmış aynı başlığı ayırır", () => {
    const apps = blockWindows(
      [span("Code", "a", -10, 5, "kum"), span("Code", "a", 5, 10), span("Code", "dışarıda", 40, 50)],
      at(0),
      at(30),
    );
    expect(apps).toHaveLength(1);
    expect(apps[0].windows.map((w) => [w.projectId, w.seconds])).toEqual([
      ["kum", 5 * 60],
      [null, 5 * 60],
    ]);
  });
});

describe("bloktaki projenin payı", () => {
  const ids = new Set(["togg"]);
  it("arada başka proje yoksa aralıkları birleştirir, varsa ayırır", () => {
    const share = projectShare(
      [
        span("Code", "a", 0, 10, "togg"),
        span("Chrome", "b", 12, 20, "togg"),
        span("Code", "c", 20, 30, "trumore"),
        span("Code", "d", 30, 40, "togg"),
        span("Code", "e", 40, 90, "trumore"),
      ],
      at(5),
      at(60),
      ids,
    );
    expect(share.seconds).toBe((5 + 8 + 10) * 60);
    expect(share.ranges).toEqual([
      [Date.parse(at(5)), Date.parse(at(20))],
      [Date.parse(at(30)), Date.parse(at(40))],
    ]);
  });

  it("projesiz pencere de aralığı böler", () => {
    const share = projectShare(
      [span("Code", "a", 0, 10, "togg"), span("Code", "b", 10, 12), span("Code", "c", 12, 20, "togg")],
      at(0),
      at(20),
      ids,
    );
    expect(share.ranges).toHaveLength(2);
  });
});
