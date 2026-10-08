import { useMemo } from "react";
import type { IdleSpan, Tag, WindowSpan, WorkBlock } from "../../api";
import { formatDuration } from "../../api";
import { addDays, fromWallMs, isoDate, today } from "../../lib/dates";
import { cn } from "../../lib/utils";
import { HOUR_PX, type ColorLens, hourRange, topFn, HourRail, Column, NowLine } from "./grid";
import { Block, IdleBlock } from "./Block";
import { gapAround, Preview, placeSessions } from "./DayCalendar";
import type { SheetSpan } from "./TimesheetLines";

/** Hafta takvimi: her gün bir sütun, bloklar kategori (ya da proje) renginde. */
export function WeekCalendar({
  from,
  blocks,
  idle = [],
  dayTotals,
  windows,
  tags,
  onSelectDay,
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
  lens = "category",
  sheet = [],
}: {
  from: Date;
  blocks: WorkBlock[];
  idle?: IdleSpan[];
  dayTotals: number[];
  windows?: WindowSpan[];
  tags: Map<string, Tag>;
  onSelectDay: (iso: string) => void;
  onEmpty?: (start: number, end: number) => void;
  onRange?: (start: number, end: number, x: number, y: number) => void;
  preview?: [number, number] | null;
  hourPx?: number;
  lens?: ColorLens;
  /** Zaman çizelgesi satırlarının aralıkları: çizelge merceğinde blokların başlığı ve rengi. */
  sheet?: SheetSpan[];
}) {
  const range = useMemo(() => hourRange(from, [...blocks, ...idle], hourPx, 7), [from, blocks, idle, hourPx]);
  const days = Array.from({ length: 7 }, (_, i) => addDays(from, i));
  const weekday = new Intl.DateTimeFormat("tr-TR", { weekday: "short" });
  const grid = "grid grid-cols-[40px_repeat(7,minmax(0,1fr))] gap-x-1";
  const now = today();

  return (
    <div>
      <div className={cn(grid, "sticky top-(--cal-head) z-20 bg-card pb-2")}>
        <span />
        {days.map((d, i) => {
          const isToday = +d === +now;
          return (
            <button
              key={i}
              onClick={() => onSelectDay(isoDate(d))}
              className="flex flex-col items-center gap-0.5 rounded-md py-1 transition-colors hover:bg-accent"
            >
              <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                {weekday.format(d)}
                <span
                  className={cn(
                    "inline-grid size-5 place-items-center rounded-full font-semibold tabular",
                    isToday ? "bg-primary text-primary-foreground" : "text-foreground",
                  )}
                >
                  {d.getDate()}
                </span>
              </span>
              <span className="text-[11px] font-medium tabular">
                {dayTotals[i] ? formatDuration(dayTotals[i]) : "—"}
              </span>
            </button>
          );
        })}
      </div>
      <div className={cn(grid, "pt-1.5")}>
        <HourRail range={range} />
        {days.map((d, i) => {
          const dayStart = +d;
          const dayEnd = +addDays(d, 1);
          const top = topFn(dayStart, range);
          return (
            <Column
              key={i}
              range={range}
              className={cn(+d === +now && "bg-primary/[0.04]")}
              onEmpty={
                onEmpty &&
                ((offset) => {
                  const gap = gapAround(fromWallMs(offset, dayStart), blocks, dayStart);
                  if (gap) onEmpty(...gap);
                })
              }
              onRange={onRange && ((a, b, x, y) => onRange(fromWallMs(a, dayStart), fromWallMs(b, dayStart), x, y))}
            >
              {placeSessions(
                blocks.filter((b) => +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd),
                idle.filter((s) => +new Date(s.start) >= dayStart && +new Date(s.start) < dayEnd),
                top,
                range.px,
              ).map(({ block, idle: span, top: t, height }) =>
                span ? (
                  <IdleBlock key={`idle-${span.start}`} span={span} onSelect={onRange} top={t} height={height} />
                ) : (
                  <Block
                    key={block.start}
                    b={block}
                    tags={tags}
                    narrow
                    lens={lens}
                    sheet={sheet}
                    windows={windows}
                    top={t}
                    height={height}
                    hourPx={range.px}
                    dayStart={dayStart}
                  />
                ),
              )}
              <Preview range={preview && preview[0] >= dayStart && preview[0] < dayEnd ? preview : null} top={top} />
              <NowLine day={d} range={range} />
            </Column>
          );
        })}
      </div>
    </div>
  );
}
