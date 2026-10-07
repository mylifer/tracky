import { describe, expect, it } from "vitest";
import type { EntryView, Report, TimesheetDay, UnassignedMeeting } from "../api";
import { buildLines } from "./DaySheet";

const meeting: UnassignedMeeting = {
  uid: "standup",
  start: new Date(2026, 9, 7, 10, 0).toISOString(),
  end: new Date(2026, 9, 7, 10, 30).toISOString(),
  subject: "Standup",
  location: "",
  online: true,
};

const entry = (start: string): EntryView => ({
  date: "2026-10-07",
  start,
  hours: 1,
  kind: "Working",
  details: "",
  party: "",
  projectId: "p1",
  division: "",
  id: null,
  key: `k-${start}`,
  exported: false,
  stale: null,
});

const day = (entries: EntryView[]): TimesheetDay => ({
  date: "2026-10-07",
  entries,
  hidden: 0,
  unassignedSeconds: 0,
  meetings: [meeting],
});

const report = { work: { blocks: [] } } as unknown as Report;

describe("çizelge görünümü satırları", () => {
  it("projesi belli olmayan toplantı birden çok çizelgede bir kez görünür", () => {
    const lines = buildLines(
      [
        { id: "a", projects: new Set(["p1"]), day: day([entry("09:00:00")]) },
        { id: "b", projects: new Set(["p2"]), day: day([]) },
      ],
      report,
    );
    expect(lines.filter((l) => l.kind === "meeting")).toHaveLength(1);
    expect(new Set(lines.map((l) => l.key)).size).toBe(lines.length);
  });

  it("satırın başlangıcı yerel saatten", () => {
    const [line] = buildLines(
      [{ id: "a", projects: new Set(["p1"]), day: { ...day([entry("14:30:15")]), meetings: [] } }],
      report,
    );
    expect(line.from).toBe(+new Date(2026, 9, 7, 14, 30, 15));
    expect(line.to - line.from).toBe(3600_000);
  });
});
