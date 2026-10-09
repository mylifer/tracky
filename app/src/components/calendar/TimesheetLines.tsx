import type { EntryView, Tag } from "../../api";
import { formatTime } from "../../lib/dates";
import { tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { HOUR_MS } from "./grid";

const hoursFmt = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** Çizgi kalınlığı ve aralığı (piksel); sütunun genişliği şerit sayısına göre. */
const LINE_PX = 4;
const LANE_PX = 8;
const PAD_PX = 6;

/** Zaman çizelgesi satırının takvimdeki aralığı: başlangıç saatinden yazılan saat kadar. */
export type SheetLine = { entry: EntryView; start: number; end: number; lane: number };

/**
 * Satırın gerçek aralıkları (ms): kapsadığı takip aralıkları; elle eklenen ya da eski satırda
 * kendi tarihi ve başlangıç saatinden yazılan saat kadar. Yazılan saat yuvarlanmış ve
 * birleştirilmiş satırda boşlukları atlamış olabilir: bloğu satırla eşlemek için gerçeği bu.
 */
export type SheetSpan = { entry: EntryView; parts: [number, number][]; length: number };

export function sheetSpans(entries: EntryView[]): SheetSpan[] {
  return entries.map((entry) => {
    let parts = (entry.coverage ?? []).filter(([a, b]) => b > a);
    if (parts.length === 0) {
      const [y, mo, d] = entry.date.split("-").map(Number);
      const [h, m] = entry.start.split(":").map(Number);
      const start = +new Date(y, mo - 1, d, h, m);
      parts = [[start, start + entry.hours * HOUR_MS]];
    }
    return { entry, parts, length: parts.reduce((t, [a, b]) => t + b - a, 0) };
  });
}

/**
 * Bloğa karşılık gelen satır: aralıkla en çok örtüşen. Yalnızca aralığın ya da satırın en az
 * yarısını kaplayan örtüşme sayılır: uzun bir bloğun ucuna değen satır bloğun adı olmaz.
 * `projectId` verilirse (projesi olan blok) yalnızca o projenin satırları: başka projenin bloğu
 * içine düşen kısa satırın adını almaz. Böyle satır yoksa `undefined`.
 */
export function sheetEntryAt(
  spans: SheetSpan[],
  start: number,
  end: number,
  projectId?: string | null,
): EntryView | undefined {
  let best: EntryView | undefined;
  let most = 0;
  for (const s of spans) {
    if (projectId && s.entry.projectId !== projectId) continue;
    const overlap = s.parts.reduce((t, [a, b]) => t + Math.max(0, Math.min(end, b) - Math.max(start, a)), 0);
    if (overlap <= 0 || overlap * 2 < Math.min(end - start, s.length)) continue;
    if (overlap > most) {
      most = overlap;
      best = s.entry;
    }
  }
  return best;
}

/**
 * Günün satırları zaman aralığına çevrilir; çakışanlar yan yana şeritlere dizilir (her satır,
 * önceki satırı bitmiş ilk şeride).
 */
export function sheetLines(entries: EntryView[], from: Date): SheetLine[] {
  const lanes: number[] = [];
  return entries
    .map((entry) => {
      const [h, m] = entry.start.split(":").map(Number);
      const start = +new Date(from.getFullYear(), from.getMonth(), from.getDate(), h, m);
      return { entry, start, end: start + entry.hours * HOUR_MS };
    })
    .sort((a, b) => a.start - b.start || b.end - a.end)
    .map((l) => {
      let lane = lanes.findIndex((end) => end <= l.start);
      if (lane < 0) lane = lanes.length;
      lanes[lane] = l.end;
      return { ...l, lane };
    });
}

/** Sütunun genişliği (piksel). */
export function sheetLinesWidth(lines: SheetLine[]) {
  const lanes = Math.max(1, ...lines.map((l) => l.lane + 1));
  return PAD_PX * 2 + (lanes - 1) * LANE_PX + LINE_PX;
}

/**
 * Zaman çizelgesi sütunu: her satır proje renginde dikey bir çizgi. Gönderilen satır dolu,
 * henüz gönderilmeyen soluk; ayrıntı ipucunda.
 */
export function TimesheetLines({
  lines,
  tags,
  top,
}: {
  lines: SheetLine[];
  tags: Map<string, Tag>;
  top: (t: number) => number;
}) {
  return lines.map(({ entry, start, end, lane }) => {
    const project = tags.get(entry.projectId);
    const t = top(start);
    const tip = [
      `${project?.name ?? "Proje"} · ${formatTime(new Date(start))}–${formatTime(new Date(end))}`,
      `${hoursFmt.format(entry.hours)} sa · ${entry.kind}`,
      entry.details,
      entry.exported ? "Gönderildi" : entry.id ? "Kaydedildi, gönderilmedi" : "Takipten, gönderilmedi",
    ]
      .filter(Boolean)
      .join("\n");
    return (
      <span
        key={entry.key}
        role="img"
        className={cn(
          "absolute rounded-full transition-[width,margin] hover:-ml-px hover:w-1.5",
          !entry.exported && "opacity-55",
        )}
        style={{
          top: t,
          height: Math.max(LINE_PX, top(end) - t - 1),
          left: PAD_PX + lane * LANE_PX,
          width: LINE_PX,
          background: tagColor(project),
        }}
        title={tip}
        aria-label={tip}
      />
    );
  });
}
