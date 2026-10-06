import { useCallback, useEffect, useState } from "react";
import { api, type Budgets, type Client, type Rule, type Tag } from "../api";
import { addDays, daysInMonth, isoDate, startOfMonth, startOfWeek, today } from "./dates";
import { friendlyError, useChanged } from "./feedback";

/** Müşteriler ve Projeler sayfalarındaki dönem seçimi. */
export type Period = "week" | "month" | "quarter" | "year";

export const PERIODS: { id: Period; label: string }[] = [
  { id: "week", label: "Bu hafta" },
  { id: "month", label: "Bu ay" },
  { id: "quarter", label: "Son 3 ay" },
  { id: "year", label: "Son 12 ay" },
];

/** Dönemin cümle içindeki adı ("bu hafta 12sa"). */
export const PERIOD_WORD: Record<Period, string> = {
  week: "bu hafta",
  month: "bu ay",
  quarter: "son 3 ay",
  year: "son 12 ay",
};

const hoursFmt = new Intl.NumberFormat("tr-TR", { maximumFractionDigits: 2 });
/** Zaman çizelgesi saati (ondalık): "7,5 sa". */
export const formatHours = (h: number) => `${hoursFmt.format(h)} sa`;

/** Grafiklerdeki hafta sayısı. */
export const WEEKS = 26;

/** Rapor aralığı: `start` gününden `days` gün; `until` verilirse o anda kesilir. */
export type Range = { start: Date; days: number; until?: Date };

/**
 * Dönemin ve kıyaslandığı önceki dönemin aralığı. Süren dönem, önceki dönemin aynı
 * noktasıyla kıyaslanır (ayın 6'sı, geçen ayın 6'sıyla): yarım dönem tam dönemle
 * kıyaslanıp hep düşüş gibi görünmesin.
 */
export function periodRanges(period: Period, now: Date): { cur: Range; prev: Range; versus: string } {
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  switch (period) {
    case "week": {
      const start = startOfWeek(day);
      return {
        cur: { start, days: 7 },
        prev: { start: addDays(start, -7), days: 7, until: shiftDays(now, -7) },
        versus: "geçen haftanın aynı anına göre",
      };
    }
    case "month": {
      const start = startOfMonth(day);
      const prevStart = new Date(start.getFullYear(), start.getMonth() - 1, 1);
      const prevDay = Math.min(now.getDate(), daysInMonth(prevStart));
      const until = new Date(prevStart);
      until.setDate(prevDay);
      until.setHours(now.getHours(), now.getMinutes(), now.getSeconds());
      return {
        cur: { start, days: daysInMonth(start) },
        prev: { start: prevStart, days: daysInMonth(prevStart), until },
        versus: "geçen ayın aynı gününe göre",
      };
    }
    case "quarter":
    case "year": {
      const days = period === "quarter" ? 91 : 365;
      const start = addDays(day, 1 - days);
      return {
        cur: { start, days },
        prev: { start: addDays(start, -days), days, until: shiftDays(now, -days) },
        versus: period === "quarter" ? "önceki 3 aya göre" : "önceki 12 aya göre",
      };
    }
  }
}

/** Aynı duvar saatinde `n` gün önce/sonra (yaz saati geçişinde de). */
function shiftDays(t: Date, n: number): Date {
  const r = new Date(t);
  r.setDate(r.getDate() + n);
  return r;
}

/** Değişim oranı; önceki dönem boşsa `null` ("yeni"), iki dönem de boşsa 0. */
export function change(cur: number, prev: number): number | null {
  if (prev > 0) return (cur - prev) / prev;
  return cur > 0 ? null : 0;
}

/** `ids` için haftalık serilerin toplamı. */
export function sumWeekly(weekly: Map<string, number[]>, ids: string[], weeks: number): number[] {
  const out = new Array<number>(weeks).fill(0);
  for (const id of ids) weekly.get(id)?.forEach((v, i) => (out[i] += v));
  return out;
}

export function sumOf(map: Map<string, number>, ids: string[]): number {
  return ids.reduce((s, id) => s + (map.get(id) ?? 0), 0);
}

export type Insights = {
  tags: Tag[];
  /** Arşivdekiler dahil tüm projeler (toplamlara sayılırlar). */
  projects: Tag[];
  clients: Client[];
  /** Proje → müşteri. */
  links: Record<string, string>;
  rules: Rule[];
  budgets: Budgets | null;
  /** Hafta başları (eskiden yeniye); son hafta süren hafta. */
  periods: string[];
  /** Proje → haftalık süre (saniye), `periods` sırasıyla. */
  weekly: Map<string, number[]>;
  /** Proje → seçilen dönemdeki süre (saniye). */
  cur: Map<string, number>;
  /** Proje → önceki dönemin aynı noktasına kadarki süre. */
  prev: Map<string, number>;
  /** Proje → bu ay zaman çizelgesine yazılan saat; ayda çizelge kaydı yoksa `null`. */
  billed: Map<string, number> | null;
  versus: string;
};

/** Müşteriler ve Projeler sayfalarının verisi; veri değişince yeniden yüklenir. */
export function useInsights(period: Period) {
  const [data, setData] = useState<Insights | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const reload = useCallback(() => setVersion((v) => v + 1), []);
  useChanged(reload);

  useEffect(() => {
    let live = true;
    const { cur, prev, versus } = periodRanges(period, new Date());
    const report = (r: Range) => api.report(isoDate(r.start), r.days, false, r.until?.toISOString());
    const toMap = (buckets: { id: string | null; seconds: number }[]) =>
      new Map(buckets.flatMap((b) => (b.id ? [[b.id, b.seconds] as const] : [])));
    Promise.all([
      api.taxonomy(),
      api.trends(WEEKS),
      api.budgets().catch(() => null),
      report(cur),
      report(prev),
      api.clientReport(isoDate(startOfMonth(today())), null, "timesheet").catch(() => null),
    ]).then(
      ([t, trends, budgets, c, p, sheet]) => {
        if (!live) return;
        const weekly = new Map<string, number[]>();
        for (const s of trends.projects) if (s.id) weekly.set(s.id, s.seconds);
        let billed: Map<string, number> | null = null;
        if (sheet && sheet.source === "timesheet" && sheet.timesheetAvailable) {
          billed = new Map();
          for (const r of sheet.rows)
            if (r.projectId) billed.set(r.projectId, (billed.get(r.projectId) ?? 0) + r.total);
        }
        setData({
          tags: t.tags,
          projects: t.tags.filter((x) => x.kind === "project"),
          clients: t.clients,
          links: t.projectClients,
          rules: t.rules,
          budgets,
          periods: trends.periods,
          weekly,
          cur: toMap(c.projects),
          prev: toMap(p.projects),
          billed,
          versus,
        });
        setError(null);
      },
      (e) => live && setError(friendlyError(e)),
    );
    return () => {
      live = false;
    };
  }, [period, version]);

  return { data, error, reload };
}
