import { useState } from "react";
import { Check, CircleDashed, Send, TriangleAlert } from "lucide-react";
import type { EntryView, Tag, WorkBlock } from "../../api";
import { formatDuration } from "../../api";
import { sendBlock } from "../../lib/blockSheet";
import { formatTime, isoDate } from "../../lib/dates";
import { friendlyError, toast } from "../../lib/feedback";
import { tagColor, tagInk } from "../../lib/tags";
import { isStale } from "../../lib/timesheet";
import { cn } from "../../lib/utils";
import { useEdit } from "../SessionEdit";
import { FULL_LABEL_PX, HOUR_MS, LABEL_MIN_PX } from "./grid";

const hoursFmt = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/**
 * Yazılan saat çeyreğe yuvarlandığı için satır sonrakine biraz taşabilir: bu kadarlık örtüşmede
 * sonraki satır aynı şeritte, öncekinin üstünde durur (yan şeride kaymaz, kartlar daralmaz).
 */
const OVERLAP_MS = 15 * 60_000;

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
 * Zaman çizelgesi sütununda bir öğe: satır (başlangıç saatinden yazılan saat kadar) ya da
 * çizelgeye gönderilmemiş blok (kendi aralığında). Çakışanlar yan yana şeritlere dizilir.
 */
export type SheetItem = { key: string; start: number; end: number; lane: number } & (
  { entry: EntryView; block?: undefined } | { block: WorkBlock; entry?: undefined }
);

/** Gönderilmemiş bloğun sütundaki anahtarı (satırınki `entry.key`). */
export const unsentKey = (b: Pick<WorkBlock, "start">) => `gonderilmedi:${b.start}`;

/**
 * Günün satırları, zaman sırasıyla şeritlere dizilmiş (`lanes` şerit sayısı), ve projesi çizelgeye
 * bağlı olup gönderilmemiş blokları (`sent === false`). Gönderilmemiş bloklar şeride girmez: satırların
 * arkasında tam genişlikte durur (içindeki toplantı satırı kartları daraltmasın) ve önce gelir.
 * Başka günün satırları ve blokları atlanır (hafta görünümü dönemin hepsini verir).
 */
export function sheetItems(entries: EntryView[], blocks: WorkBlock[], from: Date) {
  const day = isoDate(from);
  const dayStart = +from;
  const dayEnd = +new Date(from.getFullYear(), from.getMonth(), from.getDate() + 1);
  const unsent: SheetItem[] = blocks
    .filter((b) => b.sent === false && +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd)
    .map((block) => ({
      block,
      key: unsentKey(block),
      start: +new Date(block.start),
      end: +new Date(block.end),
      lane: 0,
    }))
    .sort((a, b) => a.start - b.start);
  const ends: number[] = [];
  const rows: SheetItem[] = entries
    .filter((entry) => entry.date === day)
    .map((entry) => {
      const [h, m] = entry.start.split(":").map(Number);
      const start = +new Date(from.getFullYear(), from.getMonth(), from.getDate(), h, m);
      return { entry, key: entry.key, start, end: start + entry.hours * HOUR_MS };
    })
    .sort((a, b) => a.start - b.start || b.end - a.end)
    .map((item) => {
      let lane = ends.findIndex((end) => end - OVERLAP_MS <= item.start);
      if (lane < 0) lane = ends.length;
      ends[lane] = Math.max(ends[lane] ?? 0, item.end);
      return { ...item, lane };
    });
  return { items: [...unsent, ...rows], lanes: Math.max(1, ends.length) };
}

/**
 * Satır ile takvimdeki bloğunu birlikte işaretler (biri üstüne gelince): takvimin kökündeki
 * (`data-sheet-root`) aynı `data-sheet-key`li öğeler `data-linked` alır.
 */
export function linkHover(el: HTMLElement, key: string | undefined, on: boolean) {
  if (!key) return;
  el.closest("[data-sheet-root]")
    ?.querySelectorAll(`[data-sheet-key="${CSS.escape(key)}"]`)
    .forEach((n) => n !== el && n.toggleAttribute("data-linked", on));
}

const hover = (key: string) => ({
  "data-sheet-key": key,
  onPointerEnter: (e: React.PointerEvent<HTMLElement>) => linkHover(e.currentTarget, key, true),
  onPointerLeave: (e: React.PointerEvent<HTMLElement>) => linkHover(e.currentTarget, key, false),
});

/** Şeridine göre yatay yer (sütunun yüzdesi). */
const laneStyle = (lane: number, lanes: number, gap: number) => ({
  left: `calc(${lane} * 100% / ${lanes})`,
  width: `calc(100% / ${lanes} - ${gap}px)`,
});

/** Satırın durumu: dosyaya gönderildi, çizelgede (kaydedildi ya da takipten) ya da takipte değişti. */
function rowStatus(e: EntryView) {
  if (isStale(e))
    return {
      icon: <TriangleAlert className="size-3 shrink-0 text-amber-600 dark:text-amber-400" />,
      text: "Takipte değişti: işin bir kısmı raporda başka projeye alınmış",
    };
  if (e.exported) return { icon: <Check className="size-3 shrink-0 text-success" />, text: "Dosyaya gönderildi" };
  return {
    icon: (
      <span className="grid size-3 shrink-0 place-items-center">
        <span className="size-1.5 rounded-full bg-foreground/60" />
      </span>
    ),
    text: e.id ? "Kaydedildi, dosyaya gönderilmedi" : "Takipten, dosyaya gönderilmedi",
  };
}

function rowTip(entry: EntryView, project: Tag | undefined, start: number, end: number) {
  return [
    `${project?.name ?? "Proje"} · ${formatTime(new Date(start))}–${formatTime(new Date(end))}`,
    `${hoursFmt.format(entry.hours)} sa · ${entry.kind}`,
    entry.details,
    [entry.party, entry.division].filter(Boolean).join(" · "),
    rowStatus(entry).text,
  ]
    .filter(Boolean)
    .join("\n");
}

/**
 * Gün takviminin zaman çizelgesi sütunu: her satır yazıldığı aralıkta (başlangıç + yazılan saat)
 * bir kart; durumu, saati, türü, açıklaması, tarafı ve birimiyle. Gönderilmemiş blok kesikli kart,
 * tıklayınca çizelgeye gönderilir. Üstüne gelince takvimdeki bloğu da işaretlenir.
 */
export function SheetColumn({
  items,
  lanes,
  tags,
  top,
}: {
  items: SheetItem[];
  lanes: number;
  tags: Map<string, Tag>;
  top: (t: number) => number;
}) {
  return items.map((it) => {
    const t = top(it.start);
    const style = {
      top: t,
      height: Math.max(4, top(it.end) - t - 1),
      ...(it.entry ? laneStyle(it.lane, lanes, 2) : laneStyle(0, 1, 2)),
    };
    return it.entry ? (
      <RowCard key={it.key} item={it} entry={it.entry} project={tags.get(it.entry.projectId)} style={style} />
    ) : (
      <UnsentCard
        key={it.key}
        item={it}
        block={it.block}
        project={it.block.projectId ? tags.get(it.block.projectId) : undefined}
        style={style}
      />
    );
  });
}

type CardStyle = { top: number; height: number; left: string; width: string };

function RowCard({
  item,
  entry,
  project,
  style,
}: {
  item: SheetItem;
  entry: EntryView;
  project?: Tag;
  style: CardStyle;
}) {
  const status = rowStatus(entry);
  const name = entry.details.trim();
  const who = [entry.party, entry.division].filter(Boolean).join(" · ");
  const hours = hoursFmt.format(entry.hours);
  return (
    <div
      {...hover(item.key)}
      className="absolute overflow-hidden rounded-[5px] border border-l-[3px] bg-card px-1.5 text-[10px] leading-tight shadow-xs data-[linked]:ring-2 data-[linked]:ring-primary"
      style={{ ...style, borderLeftColor: tagColor(project) }}
      title={rowTip(entry, project, item.start, item.end)}
    >
      {style.height >= FULL_LABEL_PX ? (
        <span className="flex h-full flex-col gap-px py-1">
          <span className="flex min-w-0 items-center gap-1 text-muted-foreground tabular">
            {status.icon}
            <span className="truncate">
              {hours} sa · {entry.kind} · {formatTime(new Date(item.start))}–{formatTime(new Date(item.end))}
            </span>
          </span>
          <span
            className={cn(
              "line-clamp-2 text-[11px] font-semibold",
              !name && "font-normal text-muted-foreground italic",
            )}
          >
            {name || "Açıklama yok"}
          </span>
          {style.height >= 64 && who && <span className="truncate text-muted-foreground">{who}</span>}
        </span>
      ) : (
        style.height >= LABEL_MIN_PX && (
          <span className="flex h-full min-w-0 items-center gap-1">
            {status.icon}
            <span className="shrink-0 font-semibold tabular">{hours}</span>
            <span className={cn("truncate", !name && "text-muted-foreground italic")}>{name || "Açıklama yok"}</span>
          </span>
        )
      )}
    </div>
  );
}

function UnsentCard({
  item,
  block,
  project,
  style,
}: {
  item: SheetItem;
  block: WorkBlock;
  project?: Tag;
  style: CardStyle;
}) {
  const edit = useEdit();
  const [busy, setBusy] = useState(false);
  const time = `${formatTime(new Date(item.start))}–${formatTime(new Date(item.end))}`;
  const tip = [
    `${project?.name ?? "Proje"} · ${time} · ${formatDuration(block.activeSeconds)}`,
    "Bu blok zaman çizelgesine gönderilmedi",
    edit && "Tıkla: zaman çizelgesine gönder (satırı blok değiştikçe değişir)",
  ]
    .filter(Boolean)
    .join("\n");
  async function send() {
    if (!edit) return;
    setBusy(true);
    try {
      await sendBlock(block, edit.onChanged);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(false);
    }
  }
  return (
    <button
      type="button"
      {...hover(item.key)}
      disabled={!edit || busy}
      className="absolute overflow-hidden rounded-[5px] border border-dashed border-muted-foreground/45 px-1.5 text-left text-[10px] leading-tight text-muted-foreground enabled:cursor-pointer enabled:hover:border-primary/60 enabled:hover:text-foreground data-[linked]:ring-2 data-[linked]:ring-primary"
      style={style}
      title={tip}
      aria-label={tip}
      onClick={() => void send()}
    >
      {style.height >= FULL_LABEL_PX ? (
        <span className="flex h-full flex-col items-start gap-1 py-1">
          <span className="flex max-w-full min-w-0 items-center gap-1">
            <CircleDashed className="size-3 shrink-0" />
            <span className="truncate">Gönderilmedi · {formatDuration(block.activeSeconds)}</span>
          </span>
          {edit && style.height >= 56 && (
            <span className="inline-flex items-center gap-1 rounded-[4px] border bg-card px-1.5 py-px font-medium text-foreground">
              <Send className="size-2.5 text-primary" />
              Gönder
            </span>
          )}
        </span>
      ) : (
        style.height >= LABEL_MIN_PX && (
          <span className="flex h-full min-w-0 items-center gap-1">
            <CircleDashed className="size-3 shrink-0" />
            <span className="truncate">Gönderilmedi</span>
          </span>
        )
      )}
    </button>
  );
}

/** Hafta takviminde günün yanındaki zaman çizelgesi şeridinin genişliği (piksel). */
export const SHEET_STRIP_PX = 24;

/**
 * Hafta takviminde günün zaman çizelgesi şeridi: her satır yazıldığı aralıkta bir çubuk, üstünde
 * yazılan saat; dosyaya gönderilen dolu, çizelgede bekleyen soluk, gönderilmemiş blok kesikli.
 * Açıklama ipucunda; üstüne gelince takvimdeki bloğu da işaretlenir.
 */
export function SheetStrip({
  items,
  lanes,
  tags,
  top,
}: {
  items: SheetItem[];
  lanes: number;
  tags: Map<string, Tag>;
  top: (t: number) => number;
}) {
  return items.map((it) => {
    const t = top(it.start);
    const height = Math.max(4, top(it.end) - t - 1);
    const style = { top: t, height, ...(it.entry ? laneStyle(it.lane, lanes, 1) : laneStyle(0, 1, 1)) };
    if (!it.entry)
      return (
        <span
          key={it.key}
          {...hover(it.key)}
          className="absolute rounded-[3px] border border-dashed border-muted-foreground/50 data-[linked]:ring-2 data-[linked]:ring-primary"
          style={style}
          title={`${formatTime(new Date(it.start))}–${formatTime(new Date(it.end))} · ${formatDuration(it.block.activeSeconds)}\nBu blok zaman çizelgesine gönderilmedi`}
        />
      );
    const entry = it.entry;
    const project = tags.get(entry.projectId);
    const color = tagColor(project);
    return (
      <span
        key={it.key}
        {...hover(it.key)}
        className={cn(
          "absolute overflow-hidden rounded-[3px] pt-0.5 text-center text-[9px] leading-none font-bold tracking-tighter tabular data-[linked]:ring-2 data-[linked]:ring-primary",
          isStale(entry) && "outline-2 outline-amber-500",
        )}
        style={{
          ...style,
          background: entry.exported ? color : `color-mix(in srgb, ${color} 24%, var(--card))`,
          color: entry.exported ? tagInk(project) : "var(--foreground)",
          boxShadow: entry.exported ? undefined : `inset 0 0 0 1px color-mix(in srgb, ${color} 50%, transparent)`,
        }}
        title={rowTip(entry, project, it.start, it.end)}
      >
        {height >= 13 && lanes === 1 && hoursFmt.format(entry.hours)}
      </span>
    );
  });
}
