import { describe, expect, it } from "vitest";
import type { EntryView } from "../../api";
import { sheetLines, sheetLinesWidth } from "./TimesheetLines";

function row(key: string, start: string, hours: number): EntryView {
  return {
    key,
    id: null,
    exported: false,
    stale: null,
    date: "2026-03-02",
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

describe("sheetLines", () => {
  it("satırı başlangıcından yazılan saat kadar uzatır", () => {
    const [l] = sheetLines([row("a", "09:30:00", 1.25)], day);
    expect([l.start, l.end, l.lane]).toEqual([at(9, 30), at(10, 45), 0]);
  });

  it("çakışan satırları yan şeride, bitenden sonrakini ilk şeride koyar", () => {
    const lines = sheetLines([row("c", "11:00:00", 1), row("a", "09:00:00", 2), row("b", "10:00:00", 0.5)], day);
    expect(lines.map((l) => [l.entry.key, l.lane])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 0],
    ]);
    expect(sheetLinesWidth(lines)).toBeGreaterThan(sheetLinesWidth(sheetLines([row("a", "09:00:00", 1)], day)));
  });
});
