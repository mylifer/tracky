import { describe, expect, it } from "vitest";
import type { Attendance, CalendarMeeting, Meeting } from "../api";
import { attendanceLine, blockMeeting, isAdjusted, isSkipped } from "./attendance";

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

describe("blockMeeting", () => {
  const ms = (h: number, m = 0) => +new Date(2026, 9, 5, h, m);
  const cal = (over: Partial<CalendarMeeting> = {}): CalendarMeeting => ({
    ...meeting,
    projectId: null,
    ignored: false,
    attendance: base,
    ...over,
  });

  it("names a block after the attended meeting it holds", () => {
    expect(blockMeeting([cal()], ms(9, 55), ms(11, 10))?.subject).toBe("Haftalık");
    // Toplantı uzun bloğun içinde kalsa da adını verir.
    expect(blockMeeting([cal()], ms(8), ms(13))?.subject).toBe("Haftalık");
  });

  it("ignores skipped, ignored and edge-touching meetings", () => {
    expect(blockMeeting([cal({ attendance: { ...base, status: "skipped" } })], ms(10), ms(11))).toBeNull();
    expect(blockMeeting([cal({ ignored: true })], ms(10), ms(11))).toBeNull();
    // Yarısı blokta: blok başka işle sürüyor, toplantı ad vermez.
    expect(blockMeeting([cal()], ms(9), ms(10, 30))).toBeNull();
  });

  it("uses the time the call actually took and picks the longest", () => {
    // Görüşme 10:40'ta bitti: 10:30–11:30 bloğunda yalnızca 10 dk'sı var.
    const early = cal({ attendance: { ...base, end: at(10, 40) } });
    expect(blockMeeting([early], ms(10, 30), ms(11, 30))).toBeNull();
    const other = cal({ uid: "b", subject: "Demo", start: at(10, 30), end: at(12), attendance: null });
    expect(blockMeeting([cal(), other], ms(10), ms(12))?.subject).toBe("Demo");
  });
});
