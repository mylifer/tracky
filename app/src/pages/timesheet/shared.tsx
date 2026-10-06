import { useRef, useState, type ReactNode } from "react";
import {
  formatDuration,
  type EntryKind,
  type EntryView,
  type Exported,
  type RowRef,
  type Tag,
  type Timesheet as TimesheetInfo,
  type TimesheetEntry,
} from "../../api";
import { daysInMonth, formatMonth, formatWeek, startOfMonth, startOfWeek } from "../../lib/dates";
import Toolbar from "../../components/Toolbar";
import { cn } from "../../lib/utils";

export const KINDS: EntryKind[] = ["Working", "Online", "F2F"];
export const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric", month: "short" });
export const num = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

export const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });
/** Uyarı rozeti (saat tutmuyor gibi; aktarımı engellemez). */
export const WARN_BADGE = "border-amber-500/40 text-amber-700 dark:text-amber-400";
/** Dönem denetimindeki düzeltme bağlantıları. */
export const FIX_LINK = "rounded text-muted-foreground underline-offset-2 hover:text-foreground hover:underline";
/** Satır ızgarası: seçim, başlangıç, saat, tür, açıklama, taraf, birim, sil. */
export const ROW_GRID = "grid-cols-[20px_104px_128px_96px_minmax(160px,1fr)_96px_minmax(110px,180px)_28px]";
/** Satır düzenlenirken kaydedilen alanlar (yeniden yüklemede yazılanın üzerine yazılmaz). */
export const EDITABLE = ["start", "hours", "kind", "details", "party", "division"] as const;

/** Takvimde "yoksay" seçeneğinin değeri. */
export const IGNORE = "__yoksay__";

export function exportNotice(r: Exported) {
  const where = r.sheets ? `Google Sheets'e (${r.target})` : "Excel'e";
  const skipped = r.skipped ? `, ${r.skipped} satır zaten yazılmıştı` : "";
  const backup = r.backup ? ` Yedek: ${r.backup}` : "";
  return `${r.rows} satır ${where} eklendi (${r.filled} boş satıra, ${r.inserted} yeni satır${skipped}).${backup}`;
}

export type Mode = "day" | "week" | "month";
export const MODES: { id: Mode; label: string; current: string; close: string }[] = [
  { id: "day", label: "Gün", current: "Bugün", close: "Günü kapat" },
  { id: "week", label: "Hafta", current: "Bu hafta", close: "Haftayı kapat" },
  { id: "month", label: "Ay", current: "Bu ay", close: "Ayı kapat" },
];
export const MODE_KEY = "kum.timesheet.mode";
/** Son açılan zaman çizelgesi (birden çok firma varsa). */
export const SHEET_KEY = "kum.timesheet.sheet";
const longDate = new Intl.DateTimeFormat("tr-TR", {
  weekday: "long",
  day: "numeric",
  month: "long",
  year: "numeric",
});

export function savedMode(): Mode {
  try {
    const m = localStorage.getItem(MODE_KEY);
    if (m === "day" || m === "week" || m === "month") return m;
  } catch {
    // Depolama kapalıysa varsayılan.
  }
  return "week";
}

export function savedSheet(): string | null {
  try {
    return localStorage.getItem(SHEET_KEY);
  } catch {
    return null;
  }
}

/** Görünümün ilk günü ve gün sayısı; `anchor` görünümdeki herhangi bir gün. */
export function range(mode: Mode, anchor: Date): { start: Date; days: number } {
  if (mode === "day") return { start: anchor, days: 1 };
  if (mode === "week") return { start: startOfWeek(anchor), days: 7 };
  const start = startOfMonth(anchor);
  return { start, days: daysInMonth(start) };
}

export function rangeTitle(mode: Mode, start: Date) {
  if (mode === "day") return longDate.format(start);
  if (mode === "week") return formatWeek(start);
  const m = formatMonth(start);
  return m.charAt(0).toUpperCase() + m.slice(1);
}

/** Gerçek süre (saat) → "1sa 7dk". */
export function actual(hours: number) {
  return formatDuration(Math.round(hours * 3600));
}

/** Kayıtların gerçek süresi; bilinmiyorsa yazılan saat. */
export function worked(e: TimesheetEntry) {
  return e.actualHours ?? e.hours;
}

/** Saat, adam-gün (saat / günlük saat; yuvarlanmaz). */
export function manDays(hours: number, dayHours: number) {
  return `${num.format(hours)} sa · ${num.format(hours / (dayHours || 8))} ag`;
}

/** Günlük saatten fark: "+0,50 sa", "−1,50 sa". */
export function signedHours(diff: number) {
  return `${diff > 0 ? "+" : "−"}${num.format(Math.abs(diff))} sa`;
}

export function toRef(e: EntryView): RowRef {
  return { id: e.id, entry: e };
}

/** Projenin adı; çizelgenin eşlemesindeki birim değil, Kum'daki proje. */
export function projectName(projects: Tag[], id: string) {
  return projects.find((p) => p.id === id)?.name;
}

/** Projenin satırlarına yazılan birim: eşlemedeki birim, yoksa proje adı. */
export function defaultDivision(sheet: TimesheetInfo, projects: Tag[], projectId: string) {
  const m = sheet.projects.find((x) => x.projectId === projectId);
  return m?.division.trim() || projectName(projects, projectId) || "";
}

/** Gün kartını ortaya getirir; `focusEmpty` ise ilk boş açıklamaya odaklanır. */
export function showDay(date: string, focusEmpty = false) {
  const card = document.getElementById(`gun-${date}`);
  if (!card) return;
  card.scrollIntoView({ behavior: "smooth", block: "center" });
  card.animate([{ boxShadow: "0 0 0 2px var(--ring)" }, { boxShadow: "0 0 0 0 transparent" }], { duration: 1400 });
  if (focusEmpty)
    card.querySelector<HTMLInputElement>("input[data-empty]:not(:disabled)")?.focus({ preventScroll: true });
}

/** Sağ paneldeki bölüm ve başlığı. */
export const RAIL = "space-y-3 bg-card px-4 py-3";
export const RAIL_TITLE = "text-[11px] font-semibold text-muted-foreground";
/** Kaynak satırı: simge ve ad; tıklayınca Ayarlar. */
export const SOURCE =
  "flex w-full min-w-0 items-start gap-2 text-left text-muted-foreground underline-offset-2 hover:text-foreground hover:underline";

/** Sayfanın üst çubuğu ve kayan içeriği. */
export function Page({ title, controls, children }: { title: string; controls?: ReactNode; children: ReactNode }) {
  return (
    <>
      <Toolbar title={title}>{controls}</Toolbar>
      <div className="page-enter @container flex-1 overflow-y-auto">{children}</div>
    </>
  );
}

/** Durum sayacı: değer sıfırsa sönük, bilinmiyorsa "—". */
export function Stat({
  value,
  label,
  tone,
  title,
}: {
  value: number | null;
  label: string;
  tone: string;
  title?: string;
}) {
  return (
    <div
      className={cn("flex flex-col rounded-lg px-2 py-1.5", value ? tone : "bg-muted text-muted-foreground")}
      title={title}
    >
      <span className="text-base leading-tight font-semibold tabular">{value ?? "—"}</span>
      <span className="text-[11px] opacity-80">{label}</span>
    </div>
  );
}

export type Run = (f: () => Promise<unknown>) => () => Promise<void>;

/**
 * Süren işlem: `guard(f)` işlem bitene kadar yeniden çalışmaz (çift tıklama satırı iki kez
 * eklemesin); `busy` bu sırada düğmeyi kilitler.
 */
export function useBusy() {
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const guard = (f: () => Promise<unknown>) => async () => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    try {
      await f();
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return [busy, guard] as const;
}
