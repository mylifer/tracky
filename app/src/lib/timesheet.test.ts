import { describe, expect, it } from "vitest";
import type { EntryKind, EntryView, TimesheetDay } from "../api";
import { closeReport, copyDetails, hoursDiff, mergeProblem, started } from "./timesheet";

let seq = 0;
function row(projectId: string, hours: number, details = "", kind: EntryKind = "Working", exported = false): EntryView {
  const id = `r${++seq}`;
  return {
    id,
    key: id,
    exported,
    stale: null,
    date: "2026-09-28",
    start: "09:00:00",
    hours,
    kind,
    details,
    party: "ADBA",
    projectId,
    division: projectId,
  };
}

function day(date: string, entries: EntryView[], extra: Partial<TimesheetDay> = {}): TimesheetDay {
  return { date, entries, hidden: 0, unassignedSeconds: 0, meetings: [], ...extra };
}

describe("dönemi kapatma denetimi", () => {
  // 28 Eylül 2026 pazartesi; 3–4 Ekim hafta sonu.
  const changed = { ...row("a", 1, "Rapor"), stale: 0.5 };
  const week = [
    day("2026-09-28", [row("a", 5, "Analiz"), row("b", 3, "Toplantı")]),
    day("2026-09-29", [row("a", 6.5, "")], { unassignedSeconds: 1800 }),
    day("2026-09-30", [], { unassignedSeconds: 120 }),
    day("2026-10-01", [row("a", 9, "Geliştirme")], {
      meetings: [{ uid: "m", start: "", end: "", subject: "1:1", location: "", online: true }],
    }),
    day("2026-10-02", [row("a", 2, "", "Working", true), changed, row("a", 5, "Kod")]),
    day("2026-10-03", []),
    day("2026-10-05", []),
  ];

  it("sorunları gün gün listeler, gelecek ve hafta sonuna bakmaz", () => {
    const r = closeReport(week, 8, "2026-10-04");
    expect(r.details).toEqual([{ date: "2026-09-29", rows: 1 }]);
    // Takipte değişen satır da aktarımı engeller.
    expect(r.stale).toEqual([{ date: "2026-10-02", rows: [changed] }]);
    expect(r.blocking).toBe(2);
    // 2 dakikalık atanmamış süre gürültü sayılır.
    expect(r.unassigned).toEqual([{ date: "2026-09-29", seconds: 1800 }]);
    expect(r.meetings).toEqual(["2026-10-01"]);
    expect(r.hours).toEqual([
      { date: "2026-09-29", hours: 6.5, diff: -1.5 },
      { date: "2026-10-01", hours: 9, diff: 1 },
    ]);
    expect(r.empty).toEqual(["2026-09-30"]);
    expect(r.issues).toBe(7);
  });

  it("sorun yoksa hazır", () => {
    const r = closeReport([week[0]], 8, "2026-10-04");
    expect(r.issues).toBe(0);
    expect(hoursDiff(week[0], 8)).toBe(0);
    expect(hoursDiff(week[0], 7.5)).toBeCloseTo(0.5);
    // Tamamı aktarılmış gün kapanmıştır; saatine bakılmaz.
    expect(hoursDiff(day("2026-10-02", [row("a", 2, "x", "Working", true)]), 8)).toBe(0);
  });
});

describe("toplu gönderim", () => {
  it("henüz başlamamış satırları (ileri gün, günün ilerisindeki toplantı) göndermez", () => {
    const now = new Date(2026, 8, 28, 14, 30);
    const at = (date: string, start: string) => ({ ...row("a", 1, "x"), date, start });
    expect(started(at("2026-09-25", "18:00:00"), now)).toBe(true);
    expect(started(at("2026-09-28", "14:30:00"), now)).toBe(true);
    expect(started(at("2026-09-28", "15:00:00"), now)).toBe(false);
    expect(started(at("2026-10-01", "09:00:00"), now)).toBe(false);
  });
});

describe("birleştirme", () => {
  it("aynı gün ve projenin aktarılmamış satırları birleşir", () => {
    const [a, b] = [row("a", 1, "x"), row("a", 0.5, "y")];
    expect(mergeProblem([a, b])).toBeNull();
    expect(mergeProblem([a])).toMatch(/en az iki/);
    expect(mergeProblem([a, { ...b, projectId: "b" }])).toMatch(/aynı projenin/);
    expect(mergeProblem([a, { ...b, date: "2026-09-29" }])).toMatch(/aynı günün/);
    expect(mergeProblem([a, { ...b, exported: true }])).toMatch(/Aktarılmış/);
  });
});

describe("önceki günden açıklama kopyalama", () => {
  it("aynı proje ve türdeki en yakın açıklamaları boş satırlara dağıtır", () => {
    const target = [
      row("a", 2),
      row("a", 1),
      row("a", 1),
      row("b", 1, "", "Online"),
      row("c", 1),
      row("a", 1, "Dolu"),
      row("a", 1, "", "Working", true),
    ];
    const yesterday = { entries: [row("a", 3, "Sprint planlama"), row("a", 2, "Kod inceleme"), row("b", 1, "")] };
    const older = { entries: [row("a", 1, "Eski iş"), row("b", 1, "Haftalık", "Working")] };
    const got = copyDetails(target, [yesterday, older]).map((c) => [c.entry.key, c.details]);
    expect(got).toEqual([
      [target[0].key, "Sprint planlama"],
      [target[1].key, "Kod inceleme"],
      // Kaynak bitince sonuncusu.
      [target[2].key, "Kod inceleme"],
      // Dünkü b satırı boş; daha eski günden, türü farklı olsa da aynı projeden.
      [target[3].key, "Haftalık"],
    ]);
  });

  it("canlı (kaydedilmemiş) satırlara da yazar", () => {
    const live = { ...row("a", 1), id: null, key: "a@1" };
    expect(copyDetails([live], [{ entries: [row("a", 1, "X")] }])).toEqual([{ entry: live, details: "X" }]);
  });
});
