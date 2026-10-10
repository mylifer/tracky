import type { EntryView, FileRow, TimesheetDay, TimesheetEntry } from "../api";
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

/**
 * Kaydedildikten sonra işi raporda başka projeye alınmış satır: aktarılmamışsa güncellenmeden
 * gönderilmez, aktarılmışsa güncellenince dosyadaki satırı da değişir.
 */
export function isStale(e: EntryView) {
  return e.stale != null;
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

/** Bu kadar içinde dönülen iş takipten gelen satıra eklenir (çekirdekteki `MERGE_GAP`). */
const MERGE_GAP_MS = 15 * 60_000;

/**
 * Satırın işi sürüyor mu (henüz gönderilmez). Yazılan saat yuvarlanmıştır; bitiş, kapsadığı son
 * takip aralığından. Takipten gelen (kaydedilmemiş) satıra son işinden sonra `MERGE_GAP` dolmadan
 * dönülen iş eklenir: o zamana kadar sürüyor sayılır. Kaydedilmiş satır yalnızca bitişine kadar
 * (ör. süren toplantı); aralığı olmayan (elle eklenen) satırda bitiş başlangıç + saattir.
 */
export function running(e: EntryView, now: Date) {
  if (!started(e, now)) return false;
  const parts = e.coverage ?? [];
  if (parts.length === 0) {
    const [y, mo, d] = e.date.split("-").map(Number);
    const [h, m, s] = e.start.split(":").map(Number);
    return +new Date(y, mo - 1, d, h, m, s || 0) + e.hours * 3600_000 > +now;
  }
  const end = Math.max(...parts.map(([, b]) => b));
  return end + (e.id === null ? MERGE_GAP_MS : 0) > +now;
}

/** Aktarılmamış satır gönderilemez: açıklaması boş ya da takipte değişmiş. */
export function blocked(e: EntryView) {
  return !e.exported && (needsDetails(e) || isStale(e));
}

/**
 * Günün firmaya yazılan saatinin günlük saatten farkı (+ fazla, − eksik); `extra` dosyada Kum
 * dışında girilmiş satırların saati. Hafta sonu, Kum'da kaydı olmayan ve tamamı aktarılmış
 * (kapanmış) günlerde 0.
 */
export function hoursDiff(day: TimesheetDay, dayHours: number, extra = 0) {
  if (!isWeekday(day.date) || day.entries.length === 0 || day.entries.every((e) => e.exported)) return 0;
  const diff = day.entries.reduce((s, e) => s + e.hours, 0) + extra - dayHours;
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

/**
 * Dönemin günlerini kapatmadan önce denetler; bugünden sonraki günlere bakılmaz. `extra`: gün
 * başına dosyada Kum dışında girilmiş satırların saati.
 */
export function closeReport(
  days: TimesheetDay[],
  dayHours: number,
  todayIso: string,
  extra: Map<string, number> = new Map(),
): CloseReport {
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
    const diff = hoursDiff(d, dayHours, extra.get(d.date) ?? 0);
    if (diff !== 0) r.hours.push({ date: d.date, hours: dayHours + diff, diff });
    if (isWeekday(d.date) && d.entries.length === 0 && !extra.get(d.date)) r.empty.push(d.date);
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

/** Günün dosyada Kum dışında girilmiş satırı (özet için yalnızca saati ve birimi). */
export type OutsideRow = { hours: number | null; division: string };

/** Hafta ve ay panolarında bir gün. */
export type DaySummary = {
  date: string;
  /** Firmaya yazılan saat: Kum'un satırları ve dosyada Kum dışında girilenler. */
  hours: number;
  /** Birim başına saat, büyükten küçüğe. */
  divisions: { division: string; hours: number }[];
  /** Gönderilmemiş Kum satırı sayısı. */
  unsent: number;
  /** Satırı var ve Kum'un satırlarının hepsi gönderilmiş. */
  sent: boolean;
  /** Hiç satırı yok. */
  empty: boolean;
  /** Günlük saatten fark (bugün ve öncesi; bkz. [hoursDiff]). */
  diff: number;
  /** Bakılacaklar, okunur cümlelerle. */
  problems: string[];
};

/** Günün özeti: hafta ve ay panolarında hücre, durum ve ipucu. */
export function summarizeDay(day: TimesheetDay, outside: OutsideRow[], dayHours: number, todayIso: string): DaySummary {
  const by = new Map<string, number>();
  const add = (division: string, hours: number) => by.set(division, (by.get(division) ?? 0) + hours);
  for (const e of day.entries) add(e.division, e.hours);
  for (const r of outside) add(r.division, r.hours ?? 0);
  const extra = outside.reduce((s, r) => s + (r.hours ?? 0), 0);
  const hours = [...by.values()].reduce((s, h) => s + h, 0);
  const unsent = day.entries.filter((e) => !e.exported).length;
  const blank = day.entries.filter(needsDetails).length;
  const stale = day.entries.filter(isStale).length;
  const problems: string[] = [];
  if (blank) problems.push(`${blank} satırın açıklaması boş`);
  if (stale) problems.push(`${stale} satır takipte değişti`);
  if (day.meetings.length) problems.push(`${day.meetings.length} toplantının projesi belli değil`);
  if (day.date <= todayIso && day.unassignedSeconds >= UNASSIGNED_MIN)
    problems.push(`${Math.round(day.unassignedSeconds / 60)} dk atanmamış süre`);
  return {
    date: day.date,
    hours,
    divisions: [...by.entries()]
      .filter(([, h]) => h > 0)
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], "tr"))
      .map(([division, h]) => ({ division, hours: h })),
    unsent,
    sent: day.entries.length > 0 && unsent === 0,
    empty: day.entries.length === 0 && outside.length === 0,
    diff: day.date <= todayIso ? hoursDiff(day, dayHours, extra) : 0,
    problems,
  };
}

/** Birimin rengi: listedeki sırasına göre paletten (birimin kendi rengi yok). */
export function divisionColor(divisions: string[], division: string): string {
  const i = divisions.findIndex((d) => d.toLocaleLowerCase("tr") === division.toLocaleLowerCase("tr"));
  return i < 0 ? "var(--c0)" : `var(--c${(i % 8) + 1})`;
}

/** İki dosya satırının içeriği aynı (satır numarası hariç). */
export function sameFileRow(a: FileRow, b: FileRow) {
  return (
    a.date === b.date &&
    a.start === b.start &&
    a.hours === b.hours &&
    a.kind === b.kind &&
    a.details === b.details &&
    a.party === b.party &&
    a.division === b.division
  );
}

let rowUid = 0;

/**
 * Dosya satırlarına ekranda kalıcı kimlik (`uid`) verir: satır düzenlenince ya da dosya yeniden
 * okununca kimlik değişmez, düzenlenen satır yazarken yeniden kurulmaz. Kimliği olan satır onu
 * korur (bir kez); olmayan, önceki listede içeriği aynı ve kimliği boşta olan satırınkini alır,
 * yoksa yeni kimlik alır.
 */
export function keepRowIds<T extends FileRow & { uid?: number }>(
  prev: readonly (FileRow & { uid?: number })[],
  next: readonly T[],
): (T & { uid: number })[] {
  const used = new Set<number>();
  const kept = next.map((r) => {
    if (r.uid === undefined || used.has(r.uid)) return undefined;
    used.add(r.uid);
    return r.uid;
  });
  return next.map((r, i) => {
    let uid = kept[i];
    if (uid === undefined) {
      uid = prev.find((p) => p.uid !== undefined && !used.has(p.uid) && sameFileRow(p, r))?.uid ?? ++rowUid;
      used.add(uid);
    }
    return r.uid === uid ? (r as T & { uid: number }) : { ...r, uid };
  });
}
