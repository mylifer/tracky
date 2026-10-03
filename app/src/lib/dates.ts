/** Yerel tarih yardımcıları. Rapor komutları "YYYY-MM-DD" (yerel) bekler. */

export function isoDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function parseIsoDate(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}

/** Haftanın pazartesisi. */
export function startOfWeek(d: Date): Date {
  const r = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const offset = (r.getDay() + 6) % 7;
  return addDays(r, -offset);
}

export function today(): Date {
  const n = new Date();
  return new Date(n.getFullYear(), n.getMonth(), n.getDate());
}

const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long" });
const shortDayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short" });
const dateFmt = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });

export function formatDay(d: Date): string {
  const t = today();
  if (+d === +t) return "Bugün";
  if (+d === +addDays(t, -1)) return "Dün";
  return dayFmt.format(d);
}

export const formatShortDay = (d: Date) => shortDayFmt.format(d);
export const formatDate = (d: Date) => dateFmt.format(d);
export const formatTime = (d: Date) => timeFmt.format(d);

export function formatWeek(start: Date): string {
  const end = addDays(start, 6);
  return `${formatDate(start)} – ${formatDate(end)} ${end.getFullYear()}`;
}
