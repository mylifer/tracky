import type { EntryView, TimesheetDay, TimesheetEntry } from "../api";
import { isoDate, parseIsoDate } from "./dates";

/**
 * Zaman çizelgesi denetimleri: dönemi kapatmadan (göndermeden) önce bakılacaklar, satır seçimi
 * ve önceki günlerden açıklama kopyalama. Saf işlevler; veriler sayfada zaten yüklüdür.
 */

/** Bundan kısa atanmamış süre uyarı sayılmaz (pencere geçişleri, kısa bakışlar). */
export const UNASSIGNED_MIN = 15 * 60;
/** Günün saati günlük saatten bu kadar sapmadıkça tutuyor sayılır (kayan nokta payı). */
const HOURS_EPSILON = 0.01;

export function isWeekday(iso: string) {
  const d = parseIsoDate(iso).getDay();
  return d !== 0 && d !== 6;
}

/** Açıklaması boş, aktarılmamış satır (aktarım bunları reddeder). */
export function needsDetails(e: EntryView) {
  return !e.exported && !e.details.trim();
}

/** Kaydedildikten sonra işi raporda başka projeye alınmış satır (güncellenmeden gönderilmez). */
export function isStale(e: EntryView) {
  return !e.exported && e.stale != null;
}

/**
 * Satırın işi başladı mı: geçmiş günler ve bugünün başlangıcı geçmiş satırları. Toplu gönderim
 * henüz olmamış işi (ileri tarihli ya da günün ilerisindeki toplantı) göndermez; seçilerek
 * gönderilebilir.
 */
export function started(e: TimesheetEntry, now: Date) {
  const today = isoDate(now);
  if (e.date !== today) return e.date < today;
  const hm = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  return e.start.slice(0, 5) <= hm;
}

/** Satır gönderilemez: açıklaması boş ya da takipte değişmiş. */
export function blocked(e: EntryView) {
  return needsDetails(e) || isStale(e);
}

/**
 * Günün firmaya yazılan saatinin günlük saatten farkı (+ fazla, − eksik). Hafta sonu, kaydı
 * olmayan ve tamamı aktarılmış (kapanmış) günlerde 0.
 */
export function hoursDiff(day: TimesheetDay, dayHours: number) {
  if (!isWeekday(day.date) || day.entries.length === 0 || day.entries.every((e) => e.exported)) return 0;
  const diff = day.entries.reduce((s, e) => s + e.hours, 0) - dayHours;
  return Math.abs(diff) < HOURS_EPSILON ? 0 : diff;
}

export type CloseReport = {
  /** Gözden geçirilecek atanmamış süre (saniye). */
  unassigned: { date: string; seconds: number }[];
  /** Projesi belli olmayan takvim toplantıları olan günler. */
  meetings: string[];
  /** Saati günlük saatten farklı iş günleri. */
  hours: { date: string; hours: number; diff: number }[];
  /** Hiç kaydı olmayan iş günleri. */
  empty: string[];
  /** Açıklaması boş satırlar (aktarımı engeller). */
  details: { date: string; rows: number }[];
  /** Takipte değişen satırlar (aktarımı engeller). */
  stale: { date: string; rows: EntryView[] }[];
  /** Aktarımı engelleyen satır sayısı. */
  blocking: number;
  /** Toplam uyarı (gün başına). */
  issues: number;
};

/** Dönemin günlerini kapatmadan önce denetler; bugünden sonraki günlere bakılmaz. */
export function closeReport(days: TimesheetDay[], dayHours: number, todayIso: string): CloseReport {
  const r: CloseReport = {
    unassigned: [],
    meetings: [],
    hours: [],
    empty: [],
    details: [],
    stale: [],
    blocking: 0,
    issues: 0,
  };
  for (const d of days) {
    if (d.date > todayIso) continue;
    if (d.unassignedSeconds >= UNASSIGNED_MIN) r.unassigned.push({ date: d.date, seconds: d.unassignedSeconds });
    if (d.meetings.length > 0) r.meetings.push(d.date);
    const diff = hoursDiff(d, dayHours);
    if (diff !== 0) r.hours.push({ date: d.date, hours: dayHours + diff, diff });
    if (isWeekday(d.date) && d.entries.length === 0) r.empty.push(d.date);
    const blank = d.entries.filter(needsDetails).length;
    if (blank > 0) r.details.push({ date: d.date, rows: blank });
    const stale = d.entries.filter(isStale);
    if (stale.length > 0) r.stale.push({ date: d.date, rows: stale });
    r.blocking += d.entries.filter(blocked).length;
  }
  r.issues =
    r.unassigned.length + r.meetings.length + r.hours.length + r.empty.length + r.details.length + r.stale.length;
  return r;
}

/** Seçili satırlar neden birleştirilemez; birleştirilebiliyorsa `null`. */
export function mergeProblem(rows: EntryView[]): string | null {
  if (rows.length < 2) return "Birleştirmek için en az iki satır seç";
  if (rows.some((r) => r.exported)) return "Aktarılmış satır birleştirilemez";
  if (new Set(rows.map((r) => r.date)).size > 1) return "Yalnızca aynı günün satırları birleştirilir";
  if (new Set(rows.map((r) => r.projectId)).size > 1) return "Yalnızca aynı projenin satırları birleştirilir";
  return null;
}

/**
 * Önceki günlerin açıklamalarını boş açıklamalı satırlara dağıtır. `previous` yeniden eskiye
 * sıralı günlerdir; her (proje, tür) için o projede açıklaması olan en yakın gün kaynak olur,
 * aynı türde yoksa projenin herhangi bir türü. Bir projede birden çok boş satır varsa
 * kaynaktaki açıklamalar sırayla, bitince sonuncusu verilir. Yalnızca aktarılmamış satırlar
 * (kaydedilmiş ya da canlı) değişir.
 */
export function copyDetails(
  target: EntryView[],
  previous: { entries: TimesheetEntry[] }[],
): { entry: EntryView; details: string }[] {
  const sources = new Map<string, string[]>();
  const add = (key: string, entries: TimesheetEntry[]) => {
    if (sources.has(key)) return;
    const texts = [...new Set(entries.map((e) => e.details.trim()).filter(Boolean))];
    if (texts.length) sources.set(key, texts);
  };
  for (const day of previous) {
    const byKey = new Map<string, TimesheetEntry[]>();
    for (const e of day.entries)
      for (const key of [`${e.projectId}|${e.kind}`, e.projectId]) byKey.set(key, [...(byKey.get(key) ?? []), e]);
    for (const [key, entries] of byKey) add(key, entries);
  }
  const used = new Map<string, number>();
  const out: { entry: EntryView; details: string }[] = [];
  for (const e of target) {
    if (!needsDetails(e)) continue;
    const key = sources.has(`${e.projectId}|${e.kind}`) ? `${e.projectId}|${e.kind}` : e.projectId;
    const texts = sources.get(key);
    if (!texts) continue;
    const i = used.get(key) ?? 0;
    used.set(key, i + 1);
    out.push({ entry: e, details: texts[Math.min(i, texts.length - 1)] });
  }
  return out;
}
