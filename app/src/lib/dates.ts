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

/** UTC farkının (dk) iki an arasındaki değişimi, ms; yaz saati geçişinde ±1 saat. */
function shiftMs(t: number, dayStart: number): number {
  return (new Date(t).getTimezoneOffset() - new Date(dayStart).getTimezoneOffset()) * 60_000;
}

/**
 * `t` anının, `dayStart` gece yarısından itibaren duvar saati karşılığı (ms). Geçen süre yerine
 * bunu kullanmak yaz saati geçiş günlerinde blokları saat etiketleriyle hizalı tutar.
 */
export function wallMs(t: number, dayStart: number): number {
  return t - dayStart - shiftMs(t, dayStart);
}

/** `wallMs`'in tersi: günün duvar saatinden (ms) ana. */
export function fromWallMs(ms: number, dayStart: number): number {
  const t = dayStart + ms;
  return t + shiftMs(t, dayStart);
}

export function today(): Date {
  const n = new Date();
  return new Date(n.getFullYear(), n.getMonth(), n.getDate());
}

const dateFmt = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "short" });
const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });

export const formatDate = (d: Date) => dateFmt.format(d);
export const formatTime = (d: Date) => timeFmt.format(d);

export function formatWeek(start: Date): string {
  const end = addDays(start, 6);
  return `${formatDate(start)} – ${formatDate(end)} ${end.getFullYear()}`;
}

export function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

export function addMonths(d: Date, n: number): Date {
  return new Date(d.getFullYear(), d.getMonth() + n, 1);
}

export function daysInMonth(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
}

const monthFmt = new Intl.DateTimeFormat("tr-TR", { month: "long", year: "numeric" });
export const formatMonth = (d: Date) => monthFmt.format(d);
