/**
 * Toplantı katılımının metinleri: takvimdeki toplantı bloğunda ve menüsünde gösterilir.
 * Saf işlevler; karar Rust tarafında (tracky_core::attendance).
 */
import type { Attendance, CalendarMeeting, Meeting } from "../api";
import { formatTime } from "./dates";

const span = (a: string, b: string) => `${formatTime(new Date(a))}–${formatTime(new Date(b))}`;

/** Zaman çizelgesine katılmadın diye girmiyor mu? */
export function isSkipped(a: Attendance | null | undefined): boolean {
  return a?.status === "skipped";
}

/** Süresi davetten farklı mı (görüşmeye göre ya da çakışan toplantıdan dolayı)? */
export function isAdjusted(m: Meeting, a: Attendance | null | undefined): boolean {
  return (
    !!a &&
    a.status !== "skipped" &&
    (+new Date(a.start) !== +new Date(m.start) || +new Date(a.end) !== +new Date(m.end))
  );
}

/** Katılımın tek satırlık açıklaması; söylenecek bir şey yoksa `null`. */
export function attendanceLine(m: Meeting, a: Attendance | null | undefined): string | null {
  if (!a) return null;
  if (a.status === "skipped") {
    return a.auto ? `Katılmadın görünüyor${a.reason ? `: ${a.reason}` : ""}` : "Katılmadın olarak işaretlendi";
  }
  const parts: string[] = [];
  if (a.call) parts.push(`Görüşme ${span(a.call[0], a.call[1])}`);
  if (isAdjusted(m, a)) parts.push(`çizelgeye ${span(a.start, a.end)} girer`);
  if (!a.auto && a.status === "attended") parts.push("katıldın olarak işaretlendi");
  if (parts.length === 0) return null;
  const line = parts.join(" · ");
  return line.charAt(0).toUpperCase() + line.slice(1);
}

/**
 * Bloğa adını veren toplantı: katılınan (yoksayılmamış, katılmadın denmemiş) ve süresinin ya da
 * bloğun yarısından çoğu blokta geçen; birden çoksa blokta en uzun süren. Bloğun ucuna değen
 * toplantı ad vermez. Zaman çizelgesi de bloğun satırını bu adla yazar
 * (tracky_core::timesheet::named_by). Yoksa `null`.
 */
export function blockMeeting(meetings: CalendarMeeting[], start: number, end: number): CalendarMeeting | null {
  let best: CalendarMeeting | null = null;
  let most = 0;
  for (const m of meetings) {
    if (m.ignored || isSkipped(m.attendance)) continue;
    const a = +new Date(m.attendance?.start ?? m.start);
    const b = +new Date(m.attendance?.end ?? m.end);
    const inside = Math.min(b, end) - Math.max(a, start);
    if (inside <= 0 || inside * 2 <= Math.min(b - a, end - start)) continue;
    if (inside > most) [best, most] = [m, inside];
  }
  return best;
}
