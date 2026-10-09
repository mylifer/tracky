import { useEffect, useState } from "react";
import { Coffee } from "lucide-react";
import type { IdleSpan, Tag, WindowSpan, WorkBlock } from "../../api";
import { api, formatDuration } from "../../api";
import { friendlyError, toast, undoable } from "../../lib/feedback";
import { formatTime, fromWallMs, wallMs } from "../../lib/dates";
import { UNASSIGNED, UNCATEGORIZED, tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { AppIconStack } from "../AppIcon";
import { useEdit } from "../SessionEdit";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { HOUR_MS, FULL_LABEL_PX, LABEL_MIN_PX, type ColorLens, HATCH, SNAP_MS, DRAG_MIN_PX } from "./grid";
import { BlockDetails } from "./BlockDetails";
import { type SheetSpan, sheetEntryAt } from "./TimesheetLines";

/** Çizelge merceğinde zaman çizelgesinde satırı olmayan bloğun başlığı. */
const NOT_IN_SHEET = "Çizelgede yok";

function blockTitle(b: WorkBlock, tags: Map<string, Tag>, lens: ColorLens, sheet: SheetSpan[]) {
  const apps = b.topApps.map((a) => a.appName).join(", ");
  if (lens === "sheet") {
    // Başlık, bloğun aralığında zaman çizelgesinde yazan (projesi olan blokta yalnızca o projenin
    // satırı); satırı yoksa taralı ve renksiz.
    const entry = sheetEntryAt(sheet, +new Date(b.start), +new Date(b.end), b.projectId);
    const project = entry && tags.get(entry.projectId);
    return {
      color: project ? tagColor(project) : null,
      title: entry ? entry.details.trim() || project?.name || "Satır" : NOT_IN_SHEET,
      apps,
    };
  }
  const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
  const project = b.projectId ? tags.get(b.projectId) : undefined;
  return {
    // Proje merceğinde renk projeden; projesi olmayan blok taralı ve renksiz.
    color: lens === "project" ? (project ? tagColor(project) : null) : tagColor(tag),
    // Projeye atanmış blok proje adıyla görünür: atamanın sonucu takvimde hemen fark edilsin.
    title: project?.name ?? (lens === "project" ? UNASSIGNED : (tag?.name ?? b.topApps[0]?.appName ?? UNCATEGORIZED)),
    apps,
  };
}

/** Bloğa katılan, kaydı olmayan boşlukları dolduran elle kaydın adı: projesi, yoksa kategorisi ya da uygulaması. */
export function blockLabel(b: WorkBlock, tags: Map<string, Tag>) {
  return (
    (b.projectId && tags.get(b.projectId)?.name) ||
    (b.categoryId && tags.get(b.categoryId)?.name) ||
    b.topApps[0]?.appName ||
    "Çalışma"
  );
}

/** Bloğun sürüklenen kenarı ve sürüklemenin durumu (zaman damgaları ms). */
type Resize = { edge: "start" | "end"; y0: number; from: number; at: number };

/** Takvim.app tarzı etkinlik bloğu; tıklayınca ayrıntı açılır, kenarlarından uzatılır. */
export function Block({
  b,
  tags,
  top,
  height,
  hourPx,
  dayStart,
  narrow = false,
  lens = "category",
  sheet = [],
  windows,
}: {
  b: WorkBlock;
  tags: Map<string, Tag>;
  top: number;
  height: number;
  /** Bir saatin yüksekliği: kenar sürüklenirken piksel süreye çevrilir. */
  hourPx: number;
  /** Bloğun gününün başı: sürükleme ızgara gibi duvar saatiyle hesaplanır (yaz saati günleri). */
  dayStart: number;
  windows?: WindowSpan[];
  /** Dar sütun (hafta): tek satırlık blokta süre yer kaplamasın, başlık okunsun. */
  narrow?: boolean;
  lens?: ColorLens;
  /** Zaman çizelgesi satırlarının aralıkları (çizelge merceği). */
  sheet?: SheetSpan[];
}) {
  const edit = useEdit();
  const [resize, setResize] = useState<Resize | null>(null);
  // Kaydedilirken yeni sürükleme başlamasın: blok sınırları yenilenene kadar eskidir.
  const [busy, setBusy] = useState(false);
  const resizing = resize !== null;
  // Esc sürüklemeyi bırakır; blok değişmez.
  useEffect(() => {
    if (!resizing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setResize(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [resizing]);
  const { color: blockColor, title, apps } = blockTitle(b, tags, lens, sheet);
  const color = blockColor ?? "var(--c0)";
  const full = height >= FULL_LABEL_PX;
  const label = height >= LABEL_MIN_PX;
  const devices = b.devices?.map((d) => d.name).join(" + ");
  const summary = `${title} · ${formatTime(new Date(b.start))}–${formatTime(new Date(b.end))} · ${formatDuration(b.activeSeconds)}${devices ? ` · ${devices}` : ""}`;
  const start = +new Date(b.start);
  const end = +new Date(b.end);
  const picked = edit?.picked?.has(b.start) ?? false;

  /** Bloğu yeni aralığa getirir: zaman çizelgesine bu aralık gider. */
  async function commit(newStart: number, newEnd: number) {
    if (!edit) return;
    const name = blockLabel(b, tags);
    const iso = (t: number) => new Date(t).toISOString();
    setBusy(true);
    try {
      await undoable(
        api.resizeBlock(b.start, b.end, iso(newStart), iso(newEnd), name, b.categoryId, b.projectId),
        // Kısalan bloğun kesilen kısmı silinmez, ayrı blok olur.
        newStart > start || newEnd < end
          ? `Blok ikiye bölündü: ${formatTime(new Date(newStart))}–${formatTime(new Date(newEnd))}`
          : `Blok ${formatTime(new Date(newStart))}–${formatTime(new Date(newEnd))} oldu`,
      );
      edit.onChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(false);
    }
  }

  const handle = (edge: Resize["edge"]) =>
    edit &&
    !busy && (
      <span
        className={cn(
          "group/h absolute inset-x-0.5 z-[5] flex h-1.5 cursor-ns-resize justify-center",
          edge === "end" && "items-end",
        )}
        style={{ top: edge === "start" ? top : top + height - 6 }}
        title="Sürükle: bloğu uzat ya da kısaltarak ikiye böl (zaman çizelgesine yeni aralık gider)"
        aria-hidden
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          // Sütunun aralık seçimi başlamasın.
          e.stopPropagation();
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          const from = edge === "start" ? start : end;
          setResize({ edge, y0: e.clientY, from, at: from });
        }}
        onPointerMove={(e) => {
          if (!resize) return;
          // Başladığı yere dönen sürükleme bloğu değiştirmez.
          if (Math.abs(e.clientY - resize.y0) < DRAG_MIN_PX) {
            if (resize.at !== resize.from) setResize({ ...resize, at: resize.from });
            return;
          }
          const raw = fromWallMs(
            wallMs(resize.from, dayStart) + ((e.clientY - resize.y0) / hourPx) * HOUR_MS,
            dayStart,
          );
          const snap = (t: number) => Math.round(t / SNAP_MS) * SNAP_MS;
          const snapped = snap(raw);
          // Kenarın kendi dilimine oturması değişiklik sayılmaz (kenar dilim sınırında değil).
          const at = snapped === snap(resize.from) ? resize.from : snapped;
          setResize({
            ...resize,
            at:
              resize.edge === "start"
                ? Math.min(at, end - SNAP_MS)
                : Math.min(Math.max(at, start + SNAP_MS), Date.now()),
          });
        }}
        onPointerUp={() => {
          if (!resize) return;
          setResize(null);
          if (resize.at === resize.from) return;
          void (resize.edge === "start" ? commit(resize.at, end) : commit(start, resize.at));
        }}
        onPointerCancel={() => setResize(null)}
      >
        <i className="mx-auto h-[3px] w-6 rounded-full bg-foreground/40 opacity-0 transition-opacity group-hover/h:opacity-100" />
      </span>
    );

  const [newStart, newEnd] = !resize ? [start, end] : resize.edge === "start" ? [resize.at, end] : [start, resize.at];
  const wall = (t: number) => wallMs(t, dayStart);
  const ghostTop = top + ((wall(newStart) - wall(start)) / HOUR_MS) * hourPx;
  return (
    <>
      <Popover>
        <PopoverTrigger asChild>
          <button
            className={cn(
              "absolute inset-x-0.5 overflow-hidden rounded-[5px] border-l-[3px] px-1.5 text-left transition-[filter] hover:brightness-95 focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:ring-2 data-[state=open]:ring-[var(--cat)] dark:hover:brightness-125",
              resize && "opacity-50",
              picked && "ring-2 ring-primary ring-offset-1 ring-offset-card",
            )}
            style={{
              top,
              height,
              ["--cat" as string]: color,
              borderLeftColor: color,
              background: blockColor ? `color-mix(in srgb, ${color} 22%, var(--card))` : `${HATCH}, var(--card)`,
            }}
            title={edit?.onPick ? `${summary}\nShift+tıkla: seç (Shift+A seçilenleri birleştirir)` : summary}
            aria-label={summary}
            aria-pressed={edit?.onPick ? picked : undefined}
            // Shift+tık sayfadaki metni seçmesin.
            onMouseDown={(e) => e.shiftKey && edit?.onPick && e.preventDefault()}
            onClick={(e) => {
              if (!e.shiftKey || !edit?.onPick) return;
              // Ayrıntı açılmasın: Shift+tık yalnızca seçer.
              e.preventDefault();
              edit.onPick(b);
            }}
          >
            {label && (
              <span className={cn("flex h-full flex-col", full ? "py-1" : "justify-center")}>
                <span className="flex min-w-0 items-center gap-1.5">
                  <AppIconStack apps={b.topApps} size={full ? 14 : 12} max={narrow ? 1 : 3} />
                  <span className="truncate text-[11px] leading-tight font-semibold">{title}</span>
                  {!full && !narrow && (
                    <span className="ml-auto shrink-0 text-[10px] text-muted-foreground tabular">
                      {formatDuration(b.activeSeconds)}
                    </span>
                  )}
                </span>
                {full && (
                  <span className="truncate text-[10px] leading-tight text-muted-foreground tabular">
                    {formatDuration(b.activeSeconds)} · {formatTime(new Date(b.start))}–{formatTime(new Date(b.end))}
                    {apps && ` · ${apps}`}
                  </span>
                )}
              </span>
            )}
          </button>
        </PopoverTrigger>
        <PopoverContent side="right" align="start" className={windows ? "w-96" : "w-72"}>
          <BlockDetails block={b} tags={tags} windows={windows} />
        </PopoverContent>
      </Popover>
      {handle("start")}
      {handle("end")}
      {resize && resize.at !== resize.from && (
        <span
          className="pointer-events-none absolute inset-x-0.5 z-20 grid place-items-center rounded-[5px] border-2 border-dashed text-[11px] font-medium tabular"
          style={{
            top: ghostTop,
            height: Math.max(14, ((wall(newEnd) - wall(newStart)) / HOUR_MS) * hourPx - 2),
            borderColor: color,
            background: `color-mix(in srgb, ${color} 18%, transparent)`,
          }}
        >
          {formatTime(new Date(newStart))} – {formatTime(new Date(newEnd))} ·{" "}
          {formatDuration((newEnd - newStart) / 1000)}
        </span>
      )}
    </>
  );
}

/**
 * Bilgisayardan uzakta geçen süre: taralı, renksiz blok. Tıklayınca aralık menüsü açılır
 * (projeye ya da kategoriye ata, elle kayıt olarak ekle); atanan kısım çalışma süresine girer.
 */
export function IdleBlock({
  span,
  top,
  height,
  onSelect,
}: {
  span: IdleSpan;
  top: number;
  height: number;
  onSelect?: (start: number, end: number, x: number, y: number) => void;
}) {
  const a = new Date(span.start);
  const b = new Date(span.end);
  const time = `${formatTime(a)}–${formatTime(b)}`;
  const duration = formatDuration((+b - +a) / 1000);
  const tip = `Boşta · ${time} · ${duration}\nBilgisayardan uzakta geçen süre; çalışma süresine sayılmaz.`;
  return (
    <button
      type="button"
      disabled={!onSelect}
      className="absolute inset-x-0.5 overflow-hidden rounded-[5px] border border-dashed border-muted-foreground/35 px-1.5 text-left text-muted-foreground enabled:cursor-pointer enabled:hover:border-muted-foreground/60 enabled:hover:text-foreground"
      style={{
        top,
        height,
        background:
          "repeating-linear-gradient(135deg, color-mix(in srgb, var(--muted-foreground) 12%, transparent) 0 4px, transparent 4px 9px)",
      }}
      title={onSelect ? `${tip}\nTıkla: projeye ya da kategoriye ata, elle kayıt olarak ekle` : tip}
      aria-label={`Boşta ${time}`}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        const [x, y] = e.detail === 0 ? [r.right, r.top] : [e.clientX, e.clientY];
        onSelect?.(+a, +b, x, y);
      }}
    >
      {height >= LABEL_MIN_PX && (
        <span className={cn("flex h-full flex-col", height >= FULL_LABEL_PX ? "py-1" : "justify-center")}>
          <span className="flex items-center gap-1">
            <Coffee className="size-3 shrink-0" />
            <span className="truncate text-[11px] leading-tight font-medium">Boşta</span>
            {height < FULL_LABEL_PX && <span className="ml-auto shrink-0 text-[10px] tabular">{duration}</span>}
          </span>
          {height >= FULL_LABEL_PX && (
            <span className="truncate text-[10px] leading-tight tabular">{`${duration} · ${time}`}</span>
          )}
        </span>
      )}
    </button>
  );
}
