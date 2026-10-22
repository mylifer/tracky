import { describe, expect, it } from "vitest";
import type { EntryView, WorkBlock } from "../../api";
import { sheetItems, unsentKey } from "./TimesheetLines";

function row(key: string, start: string, hours: number, date = "2026-03-02"): EntryView {
  return {
    key,
    id: null,
    exported: false,
    stale: null,
    openUntil: 0,
    date,
    start,
    hours,
    kind: "Working",
    details: "",
    party: "",
    projectId: "p",
    division: "",
  };
}

const day = new Date(2026, 2, 2);
const at = (h: number, m = 0) => +new Date(2026, 2, 2, h, m);

function block(start: number, end: number, sent?: boolean): WorkBlock {
  return {
    start: new Date(start).toISOString(),
    end: new Date(end).toISOString(),
    activeSeconds: (end - start) / 1000,
    categoryId: null,
    projectId: "p",
    switches: 0,
    topApps: [],
    sent,
  };
}

const lanes = (entries: EntryView[], blocks: WorkBlock[] = []) =>
  sheetItems(entries, blocks, day).items.map((l) => [l.key, l.lane]);

describe("sheetItems", () => {
  it("satırı başlangıcından yazılan saat kadar uzatır", () => {
    const { items, lanes: n } = sheetItems([row("a", "09:30:00", 1.25)], [], day);
    expect([items[0].start, items[0].end, items[0].lane, n]).toEqual([at(9, 30), at(10, 45), 0, 1]);
  });

  it("çakışan satırları yan şeride, bitenden sonrakini ilk şeride koyar", () => {
    expect(lanes([row("c", "11:00:00", 1), row("a", "09:00:00", 2), row("b", "10:00:00", 0.5)])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 0],
    ]);
  });

  it("yuvarlamanın taşırdığı çeyrek saate kadar örtüşmede aynı şeritte kalır", () => {
    // 09:02 + 1,25 sa = 10:17; sonraki satır 10:05'te başlıyor (12 dk örtüşme).
    expect(lanes([row("a", "09:02:00", 1.25), row("b", "10:05:00", 0.5)])).toEqual([
      ["a", 0],
      ["b", 0],
    ]);
    expect(sheetItems([row("a", "09:02:00", 1.25), row("b", "10:05:00", 0.5)], [], day).lanes).toBe(1);
  });

  it("gönderilmemiş bloğu kendi aralığında ve satırlardan önce ekler; gönderileni ve çizelgesi olmayanı eklemez", () => {
    const unsent = block(at(11, 12), at(12, 38), false);
    const { items } = sheetItems(
      [row("a", "09:00:00", 1)],
      [unsent, block(at(13), at(14), true), block(at(14), at(15))],
      day,
    );
    expect(items.map((l) => [l.key, l.start, l.end])).toEqual([
      [unsentKey(unsent), at(11, 12), at(12, 38)],
      ["a", at(9), at(10)],
    ]);
  });

  it("gönderilmemiş blok içindeki satırları yan şeride itmez", () => {
    const unsent = block(at(12, 30), at(13, 50), false);
    const { items, lanes: n } = sheetItems([row("m", "13:00:00", 1)], [unsent], day);
    expect([n, items.map((l) => l.lane)]).toEqual([1, [0, 0]]);
  });

  it("başka günün satırlarını ve bloklarını atlar", () => {
    const next = +new Date(2026, 2, 3, 9);
    expect(
      lanes([row("a", "09:00:00", 1), row("b", "09:00:00", 1, "2026-03-03")], [block(next, next + 3600_000, false)]),
    ).toEqual([["a", 0]]);
  });
});
