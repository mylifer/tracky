import { useMemo } from "react";
import { Zap } from "lucide-react";
import type { Segment, Tag, WorkBlock } from "../api";
import { formatDuration } from "../api";
import { addDays, formatTime, isoDate, today } from "../lib/dates";
import { UNCATEGORIZED, tagColor, tagInk } from "../lib/tags";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";
import { BlockActions, useEdit } from "./SessionEdit";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

const HOUR_PX = 52;
const HOUR_MS = 3600_000;
/** Bu yükseklikten küçük bloklarda yalnızca başlık gösterilir. */
const FULL_LABEL_PX = 40;
/** Bundan alçak bloklarda yazı yok (yalnızca renk; ayrıntı ipucunda). */
const LABEL_MIN_PX = 15;

type Range = { first: number; last: number };

/** Etkinliğe göre gösterilecek saat aralığı (en az 08–18). */
function hourRange(from: Date, spans: { start: string; end: string }[], days = 1): Range {
  let first = 8;
  let last = 18;
  for (const s of spans) {
    const st = new Date(s.start);
    const en = new Date(s.end);
    const dayStart = new Date(st.getFullYear(), st.getMonth(), st.getDate());
    if (days === 1 && +dayStart !== +from) continue;
    first = Math.min(first, st.getHours());
    const endHour = en.getHours() + (en.getMinutes() > 0 ? 1 : 0);
    last = Math.max(last, en.getDate() !== st.getDate() ? 24 : endHour);
  }
  return { first, last: Math.min(24, Math.max(last, first + 6)) };
}

function topFn(dayStart: number, range: Range) {
  return (t: number) => ((t - (dayStart + range.first * HOUR_MS)) / HOUR_MS) * HOUR_PX;
}

function hours(range: Range) {
  const out = [];
  for (let h = range.first; h <= range.last; h++) out.push(h);
  return out;
}

function HourRail({ range }: { range: Range }) {
  return (
    <div className="relative" style={{ height: (range.last - range.first) * HOUR_PX }}>
      {hours(range).map((h) => (
        <span
          key={h}
          className="absolute right-1 -translate-y-1/2 text-[10px] text-muted-foreground tabular"
          style={{ top: (h - range.first) * HOUR_PX }}
        >
          {String(h % 24).padStart(2, "0")}:00
        </span>
      ))}
    </div>
  );
}

function Column({ range, children, className }: { range: Range; children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("relative", className)} style={{ height: (range.last - range.first) * HOUR_PX }}>
      {hours(range).map((h) => (
        <span
          key={h}
          className="absolute inset-x-0 border-t border-border/70"
          style={{ top: (h - range.first) * HOUR_PX }}
        />
      ))}
      {children}
    </div>
  );
}

function NowLine({ day, range }: { day: Date; range: Range }) {
  if (+day !== +today()) return null;
  const top = topFn(+day, range)(Date.now());
  if (top < 0 || top > (range.last - range.first) * HOUR_PX) return null;
  return (
    <span className="pointer-events-none absolute inset-x-0 z-10 h-px bg-destructive" style={{ top }}>
      <span className="absolute -top-[3px] -left-[3px] size-[7px] rounded-full bg-destructive" />
    </span>
  );
}

/** Bloğun ayrıntı kartı: kategori, süre, uygulama yüzdeleri. */
function BlockDetails({ block, tags }: { block: WorkBlock; tags: Map<string, Tag> }) {
  const edit = useEdit();
  const tag = block.categoryId ? tags.get(block.categoryId) : undefined;
  const color = tagColor(tag);
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Badge variant="outline" className="gap-1.5">
          <i className="size-2 rounded-full" style={{ background: color }} />
          {tag?.name ?? UNCATEGORIZED}
        </Badge>
        {block.focus && (
          <Badge variant="outline" className="gap-1 border-focus/30 text-focus">
            <Zap /> Odak
          </Badge>
        )}
      </div>
      <div>
        <div className="flex items-baseline justify-between gap-3">
          <strong className="truncate text-sm">{block.topApps[0]?.appName ?? tag?.name ?? "Çalışma"}</strong>
          <span className="text-sm font-semibold tabular">{formatDuration(block.activeSeconds)}</span>
        </div>
        <p className="mt-0.5 text-xs text-muted-foreground tabular">
          {formatTime(new Date(block.start))} – {formatTime(new Date(block.end))}
          {block.switches > 0 && ` · ${block.switches} uygulama geçişi`}
        </p>
      </div>
      <ul className="space-y-1.5">
        {block.topApps.map((a) => {
          const pct = block.activeSeconds ? Math.round((a.seconds / block.activeSeconds) * 100) : 0;
          return (
            <li key={a.appName} className="grid grid-cols-[34px_1fr_auto] items-center gap-2 text-xs">
              <span className="text-muted-foreground tabular">%{pct}</span>
              <span className="min-w-0">
                <span className="block truncate">{a.appName}</span>
                <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                  <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
                </span>
              </span>
              <span className="text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
            </li>
          );
        })}
      </ul>
      {edit && (
        <BlockActions
          start={block.start}
          end={block.end}
          categoryId={block.categoryId}
          categories={edit.categories}
          onChanged={edit.onChanged}
        />
      )}
    </div>
  );
}

function blockTitle(b: WorkBlock, tags: Map<string, Tag>) {
  const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
  return {
    tag,
    title: tag?.name ?? b.topApps[0]?.appName ?? UNCATEGORIZED,
    apps: b.topApps.map((a) => a.appName).join(", "),
  };
}

/** Takvim.app tarzı etkinlik bloğu; tıklayınca ayrıntı açılır. */
function Block({
  b,
  tags,
  top,
  height,
  narrow = false,
}: {
  b: WorkBlock;
  tags: Map<string, Tag>;
  top: number;
  height: number;
  /** Dar sütun (hafta): tek satırlık blokta süre yer kaplamasın, başlık okunsun. */
  narrow?: boolean;
}) {
  const { tag, title, apps } = blockTitle(b, tags);
  const color = tagColor(tag);
  const full = height >= FULL_LABEL_PX;
  const label = height >= LABEL_MIN_PX;
  const summary = `${title} · ${formatTime(new Date(b.start))}–${formatTime(new Date(b.end))} · ${formatDuration(b.activeSeconds)}`;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className="absolute inset-x-0.5 overflow-hidden rounded-[5px] border-l-[3px] px-1.5 text-left transition-[filter] hover:brightness-95 focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:ring-2 data-[state=open]:ring-[var(--cat)] dark:hover:brightness-125"
          style={{
            top,
            height,
            ["--cat" as string]: color,
            borderLeftColor: color,
            background: `color-mix(in srgb, ${color} 22%, var(--card))`,
          }}
          title={summary}
          aria-label={summary}
        >
          {label && (
            <span className={cn("flex h-full flex-col", full ? "py-1" : "justify-center")}>
              <span className="flex items-baseline gap-1.5">
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
      <PopoverContent side="right" align="start" className="w-72">
        <BlockDetails block={b} tags={tags} />
      </PopoverContent>
    </Popover>
  );
}

function blockGeometry(b: { start: string; end: string }, top: (t: number) => number) {
  const t = top(+new Date(b.start));
  return { top: t, height: Math.max(3, top(+new Date(b.end)) - t - 2) };
}

/** Gün takvimi: Oturumlar · Uygulamalar · Odak şeridi. */
export function DayCalendar({
  from,
  blocks,
  segments,
  tags,
}: {
  from: Date;
  blocks: WorkBlock[];
  segments: Segment[];
  tags: Map<string, Tag>;
}) {
  const range = useMemo(() => hourRange(from, [...blocks, ...segments]), [from, blocks, segments]);
  const top = topFn(+from, range);
  // Kısa uygulama dilimlerini aynı uygulamanın komşularıyla birleştir (takvimde okunur kalsın).
  const apps = useMemo(() => mergeSegments(segments), [segments]);
  const grid = "grid grid-cols-[40px_minmax(0,1fr)_minmax(0,1fr)_8px] gap-x-2";

  return (
    <div>
      <div className={cn(grid, "pb-2 text-[11px] font-medium text-muted-foreground")}>
        <span />
        <span>Oturumlar</span>
        <span>Uygulamalar</span>
        <span title="Odak blokları">
          <Zap className="size-3 text-focus" />
        </span>
      </div>
      <div className={cn(grid, "pt-1.5")}>
        <HourRail range={range} />
        <Column range={range}>
          {blocks.map((b) => (
            <Block key={b.start} b={b} tags={tags} {...blockGeometry(b, top)} />
          ))}
          <NowLine day={from} range={range} />
        </Column>
        <Column range={range}>
          {apps.map((s, i) => {
            const { top: t, height: h } = blockGeometry(s, top);
            const tag = s.categoryId ? tags.get(s.categoryId) : undefined;
            return (
              <span
                key={i}
                className="absolute inset-x-0.5 flex items-center overflow-hidden rounded-[4px] px-1.5 text-[10px] font-medium"
                style={{ top: t, height: h, background: tagColor(tag), color: tagInk(tag) }}
                title={`${s.appName}${s.title ? " — " + s.title : ""}\n${formatTime(new Date(s.start))}–${formatTime(new Date(s.end))} · ${formatDuration((+new Date(s.end) - +new Date(s.start)) / 1000)}`}
              >
                {h >= 18 && <span className="truncate">{s.appName}</span>}
              </span>
            );
          })}
          <NowLine day={from} range={range} />
        </Column>
        <div className="relative" style={{ height: (range.last - range.first) * HOUR_PX }}>
          {blocks.map((b) => {
            const g = blockGeometry(b, top);
            return (
              <span
                key={b.start}
                className={cn("absolute inset-x-0 rounded-full", b.focus ? "bg-focus" : "bg-muted")}
                style={{ top: g.top, height: Math.max(3, g.height - 1) }}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** Hafta takvimi: her gün bir sütun, bloklar kategori renginde. */
export function WeekCalendar({
  from,
  blocks,
  dayTotals,
  tags,
  onSelectDay,
}: {
  from: Date;
  blocks: WorkBlock[];
  dayTotals: number[];
  tags: Map<string, Tag>;
  onSelectDay: (iso: string) => void;
}) {
  const range = useMemo(() => hourRange(from, blocks, 7), [from, blocks]);
  const days = Array.from({ length: 7 }, (_, i) => addDays(from, i));
  const weekday = new Intl.DateTimeFormat("tr-TR", { weekday: "short" });
  const grid = "grid grid-cols-[40px_repeat(7,minmax(0,1fr))] gap-x-1";
  const now = today();

  return (
    <div>
      <div className={cn(grid, "pb-2")}>
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
            <Column key={i} range={range} className={cn(+d === +now && "bg-primary/[0.04]")}>
              {blocks
                .filter((b) => +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd)
                .map((b) => (
                  <Block key={b.start} b={b} tags={tags} narrow {...blockGeometry(b, top)} />
                ))}
              <NowLine day={d} range={range} />
            </Column>
          );
        })}
      </div>
    </div>
  );
}

/** Aynı uygulamanın 2 dakikadan yakın dilimlerini birleştirir. */
function mergeSegments(segments: Segment[]): Segment[] {
  const out: Segment[] = [];
  for (const s of segments) {
    const last = out[out.length - 1];
    if (last && last.appName === s.appName && +new Date(s.start) - +new Date(last.end) < 120_000) {
      last.end = s.end;
    } else {
      out.push({ ...s });
    }
  }
  return out;
}
