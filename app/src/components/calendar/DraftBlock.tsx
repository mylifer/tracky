import { useMemo, useState } from "react";
import { Combine, FolderInput, ListTree } from "lucide-react";
import type { CalendarMeeting, IdleSpan, Tag, WindowSpan } from "../../api";
import { api, formatDuration } from "../../api";
import { blockMeeting } from "../../lib/attendance";
import { type DraftGroup, asBlock } from "../../lib/draft";
import { friendlyError, toast, undoable } from "../../lib/feedback";
import { formatTime, wallMs } from "../../lib/dates";
import { UNASSIGNED, tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { AppIconStack } from "../AppIcon";
import { useEdit } from "../SessionEdit";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { AwayStripes, blockLabel, blockTitle } from "./Block";
import { FULL_LABEL_PX, HATCH, HOUR_MS, LABEL_MIN_PX, type ColorLens } from "./grid";
import type { SheetSpan } from "./TimesheetLines";
import { topWindow } from "./windowLanes";

const time = (s: string) => formatTime(new Date(s));

/**
 * Taslak: birden çok bloğun tek kutu olarak çizilişi. İçindeki araya girmeler sağ kenarda
 * çentik, boşta süre taralı şerittir. Tıklayınca parçaları listelenir: tek bloğa
 * dönüştürülebilir, aralık projeye atanabilir ya da parçalar ayrı ayrı açılabilir.
 */
export function DraftBlock({
  g,
  away,
  tags,
  lens,
  sheet,
  windows,
  meetings,
  top,
  height,
  hourPx,
  dayStart,
  onRange,
  onExpand,
}: {
  g: DraftGroup;
  away: IdleSpan[];
  tags: Map<string, Tag>;
  lens: ColorLens;
  sheet: SheetSpan[];
  windows?: WindowSpan[];
  meetings?: CalendarMeeting[];
  top: number;
  height: number;
  hourPx: number;
  dayStart: number;
  onRange?: (start: number, end: number, x: number, y: number) => void;
  onExpand: () => void;
}) {
  const edit = useEdit();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const b = useMemo(() => asBlock(g), [g]);
  const start = +new Date(g.start);
  const end = +new Date(g.end);
  const named = useMemo(() => (windows ? topWindow(windows, start, end) : null), [windows, start, end]);
  const meeting = useMemo(() => (meetings ? blockMeeting(meetings, start, end) : null), [meetings, start, end]);
  const { color: blockColor, title, project } = blockTitle(b, tags, lens, sheet, named, meeting);
  const color = blockColor ?? "var(--c0)";
  const full = height >= FULL_LABEL_PX;
  const label = height >= LABEL_MIN_PX;
  const parts = `${g.blocks.length} parça`;
  const summary = `${title}${project ? ` · ${project}` : ""} · ${time(g.start)}–${time(g.end)} · ${formatDuration(g.activeSeconds)} · ${parts}`;
  const y = (t: number) => ((wallMs(t, dayStart) - wallMs(start, dayStart)) / HOUR_MS) * hourPx;

  const partColor = (p: DraftGroup["blocks"][number]) => {
    const tag = p.projectId ? tags.get(p.projectId) : p.categoryId ? tags.get(p.categoryId) : undefined;
    return tag ? tagColor(tag) : "var(--muted-foreground)";
  };

  /** Taslağı tek bloğa çevirir: en uzun parçanın kategorisi ve projesiyle (Shift+A birleştirme gibi). */
  async function mergeAll() {
    if (!edit || busy) return;
    const main = g.blocks.reduce((a, c) => (c.activeSeconds > a.activeSeconds ? c : a));
    setBusy(true);
    try {
      await undoable(
        api.resizeBlock(main.start, main.end, g.start, g.end, blockLabel(main, tags), main.categoryId, main.projectId),
        `${g.blocks.length} parça tek blok oldu`,
      );
      setOpen(false);
      edit.onChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(false);
    }
  }

  const mainProject = (() => {
    const main = g.blocks.reduce((a, c) => (c.activeSeconds > a.activeSeconds ? c : a));
    return main.projectId ? tags.get(main.projectId)?.name : undefined;
  })();

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          className="absolute inset-x-0.5 overflow-hidden rounded-[5px] border-l-[3px] px-1.5 pr-3 text-left transition-[filter] hover:brightness-95 focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:ring-2 data-[state=open]:ring-[var(--cat)] dark:hover:brightness-125"
          style={{
            top,
            height,
            ["--cat" as string]: color,
            borderLeftColor: color,
            background: blockColor ? `color-mix(in srgb, ${color} 22%, var(--card))` : `${HATCH}, var(--card)`,
          }}
          title={`${summary}\nTıkla: parçaları gör, tek blok yap ya da projeye ata`}
          aria-label={label ? undefined : summary}
          data-draft
        >
          <AwayStripes away={away} start={start} dayStart={dayStart} hourPx={hourPx} />
          {g.others.map((o) => (
            <span
              key={o.start}
              aria-hidden
              className="pointer-events-none absolute right-1 w-1.5 rounded-[2px] opacity-80"
              style={{
                top: y(+new Date(o.start)),
                height: Math.max(2, y(+new Date(o.end)) - y(+new Date(o.start)) - 1),
                background: partColor(o),
              }}
            />
          ))}
          {label && (
            <span className={cn("relative flex h-full flex-col", full ? "py-1" : "justify-center")}>
              <span className="flex min-w-0 items-center gap-1.5">
                <AppIconStack apps={g.topApps} size={full ? 14 : 12} max={3} />
                <span className="truncate text-[11px] leading-tight font-semibold">{title}</span>
                {!full && (
                  <span className="ml-auto shrink-0 text-[10px] text-muted-foreground tabular">
                    {formatDuration(g.activeSeconds)}
                  </span>
                )}
              </span>
              {full && (
                <span className="truncate text-[10px] leading-tight text-muted-foreground tabular">
                  {project && `${project} · `}
                  {formatDuration(g.activeSeconds)} · {time(g.start)}–{time(g.end)} · {parts}
                  {g.unsent > 0 && " · gönderilmedi"}
                </span>
              )}
            </span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent side="right" align="start" className="w-80">
        <div className="space-y-3">
          <div>
            <div className="flex items-center justify-between gap-3">
              <strong className="truncate text-sm">{title}</strong>
              <span className="text-sm font-semibold tabular">{formatDuration(g.activeSeconds)}</span>
            </div>
            <p className="mt-0.5 text-xs text-muted-foreground tabular">
              {time(g.start)} – {time(g.end)} · {parts}
              {g.others.length > 0 && ` · ${g.others.length} araya giren`}
            </p>
          </div>
          <ul className="max-h-60 space-y-1 overflow-y-auto text-xs">
            {g.blocks.map((p) => {
              const name =
                (p.projectId && tags.get(p.projectId)?.name) ||
                (windows && topWindow(windows, +new Date(p.start), +new Date(p.end))?.title) ||
                p.topApps[0]?.appName ||
                UNASSIGNED;
              return (
                <li key={p.start} className="grid grid-cols-[8px_74px_minmax(0,1fr)_auto] items-center gap-2">
                  <i className="size-2 rounded-full" style={{ background: partColor(p) }} />
                  <span className="text-muted-foreground tabular">
                    {time(p.start)}–{time(p.end)}
                  </span>
                  <span className="truncate" title={name}>
                    {name}
                    {p.sent === false && <span className="ml-1 text-muted-foreground">· gönderilmedi</span>}
                  </span>
                  <span className="text-muted-foreground tabular">{formatDuration(p.activeSeconds)}</span>
                </li>
              );
            })}
          </ul>
          <div className="flex flex-wrap gap-1.5">
            {edit && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                title={`Parçalar en uzun parçanın ${mainProject ? `projesiyle (${mainProject})` : "kategorisiyle"} tek blok olur; zaman çizelgesine tek satır gider`}
                onClick={() => void mergeAll()}
              >
                <Combine /> Tek blok yap
              </Button>
            )}
            {onRange && (
              <Button
                size="sm"
                variant="outline"
                title="Aralığın tamamını bir projeye ya da kategoriye ata"
                onClick={(e) => {
                  setOpen(false);
                  onRange(start, end, e.clientX, e.clientY);
                }}
              >
                <FolderInput /> Projeye ata
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              title="Parçaları takvimde ayrı bloklar olarak göster"
              onClick={() => {
                setOpen(false);
                onExpand();
              }}
            >
              <ListTree /> Parçaları aç
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
