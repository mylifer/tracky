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
  agenda: "",
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

  it("satırın bloğu yazılan saat değil işin gerçek aralığı (takvimdeki blok)", () => {
    const at = (h: number, m: number) => +new Date(2026, 9, 7, h, m);
    // 07:00'den 3 saat yazılmış, iş 07:08–08:00 ve 09:30–10:30 arasında.
    const e = {
      ...entry("07:00:00"),
      hours: 3,
      coverage: [
        [at(7, 8), at(8, 0)],
        [at(9, 30), at(10, 30)],
      ] as [number, number][],
    };
    const [line] = buildLines([{ id: "a", projects: new Set(["p1"]), day: { ...day([e]), meetings: [] } }], report);
    expect([line.from, line.to]).toEqual([at(7, 8), at(10, 30)]);
  });

  it("toplantı satırının içindeki projesiz blok ayrıca listelenmez (süresi çizelgede)", () => {
    const at = (h: number, m: number) => new Date(2026, 9, 7, h, m).toISOString();
    const meetingRow = {
      ...entry("10:00:00"),
      kind: "Online" as const,
      hours: 0.5,
      coverage: [[+new Date(at(10, 0)), +new Date(at(10, 30))]] as [number, number][],
    };
    const blocks = {
      work: {
        blocks: [
          { start: at(10, 5), end: at(10, 28), projectId: null, topApps: [] },
          { start: at(11, 0), end: at(11, 40), projectId: null, topApps: [] },
        ],
      },
    } as unknown as Report;
    const lines = buildLines(
      [{ id: "a", projects: new Set(["p1"]), day: { ...day([meetingRow]), meetings: [] } }],
      blocks,
    );
    expect(lines.filter((l) => l.kind === "unassigned").map((l) => l.from)).toEqual([+new Date(at(11, 0))]);
  });
});
