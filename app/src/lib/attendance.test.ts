import { describe, expect, it } from "vitest";
import type { Attendance, Meeting } from "../api";
import { attendanceLine, isAdjusted, isSkipped } from "./attendance";

const at = (h: number, m = 0) => new Date(2026, 9, 5, h, m).toISOString();
const meeting: Meeting = {
  uid: "plan",
  start: at(10),
  end: at(11),
  subject: "Haftalık",
  location: "",
  online: true,
  agenda: "",
};
const base: Attendance = {
  key: "plan@1",
  status: "unknown",
  auto: true,
  start: meeting.start,
  end: meeting.end,
  call: null,
  reason: null,
};

describe("attendance", () => {
  it("says nothing when the meeting is as invited", () => {
    expect(attendanceLine(meeting, base)).toBeNull();
    expect(attendanceLine(meeting, null)).toBeNull();
  });

  it("explains skipped meetings", () => {
    const skipped = { ...base, status: "skipped" as const, reason: "Görüşme yoktu, başka işte çalıştın" };
    expect(isSkipped(skipped)).toBe(true);
    expect(attendanceLine(meeting, skipped)).toBe("Katılmadın görünüyor: Görüşme yoktu, başka işte çalıştın");
    expect(attendanceLine(meeting, { ...skipped, auto: false })).toBe("Katılmadın olarak işaretlendi");
  });

  it("shows the call and the adjusted span", () => {
    const early = {
      ...base,
      status: "attended" as const,
      end: at(10, 35),
      call: [at(9, 58), at(10, 35)] as [string, string],
    };
    expect(isAdjusted(meeting, early)).toBe(true);
    expect(attendanceLine(meeting, early)).toBe("Görüşme 09:58–10:35 · çizelgeye 10:00–10:35 girer");
    const answered = { ...base, status: "attended" as const, auto: false };
    expect(attendanceLine(meeting, answered)).toBe("Katıldın olarak işaretlendi");
  });
});
