import { describe, expect, it } from "vitest";
import { detailRows, MINOR_SECS, onlyOther } from "./minorWindows";

type W = { title: string; seconds: number; project?: string; domain?: string };
const rows = (ws: W[], max?: number) =>
  detailRows(
    ws,
    (w) => w.seconds,
    (w) =>
      w.project
        ? { key: `p:${w.project}`, label: w.project, projectId: w.project }
        : w.domain
          ? { key: `d:${w.domain}`, label: w.domain, domain: w.domain }
          : null,
    "pencere",
    max,
  ).map((r) => (r.kind === "item" ? [r.item.title, r.seconds] : [r.label, r.seconds]));

describe("ayrıntıdaki kısa pencereler", () => {
  it("5 dakikadan kısa pencereler adıyla görünmez, Diğer'de toplanır", () => {
    expect(
      rows([
        { title: "uzun", seconds: 20 * 60 },
        { title: "kısa 1", seconds: 60 },
        { title: "kısa 2", seconds: 120 },
        { title: "tam 5", seconds: MINOR_SECS },
      ]),
    ).toEqual([
      ["uzun", 1200],
      ["tam 5", 300],
      ["Diğer · 2 pencere", 180],
    ]);
  });

  it("aynı projede ya da sitede birlikte 5 dakikayı bulan kısa pencereler tek satırda", () => {
    expect(
      rows([
        { title: "a", seconds: 120, project: "Loyalty" },
        { title: "b", seconds: 120, project: "Loyalty" },
        { title: "c", seconds: 90, project: "Loyalty" },
        { title: "d", seconds: 200, domain: "jira.firma.com" },
        { title: "e", seconds: 200, domain: "jira.firma.com" },
        { title: "f", seconds: 60, project: "Kum" },
        { title: "g", seconds: 60, project: "Kum" },
        { title: "h", seconds: 280, domain: "x.com" },
      ]),
    ).toEqual([
      ["jira.firma.com · 2 pencere", 400],
      ["Loyalty · 3 pencere", 330],
      ["Diğer · 3 pencere", 400],
    ]);
  });

  it("satır sınırını aşan kısım da Diğer'e katılır", () => {
    const ws = Array.from({ length: 5 }, (_, i) => ({ title: `p${i}`, seconds: (10 - i) * 60 }));
    expect(rows(ws, 3)).toEqual([
      ["p0", 600],
      ["p1", 540],
      ["Diğer · 3 pencere", 8 * 60 + 7 * 60 + 6 * 60],
    ]);
  });

  it("yalnızca Diğer kalan liste gösterilmeye değmez", () => {
    expect(onlyOther(detailRows([{ s: 60 }], (w) => w.s))).toBe(true);
    expect(onlyOther(detailRows([{ s: 600 }], (w) => w.s))).toBe(false);
  });
});
