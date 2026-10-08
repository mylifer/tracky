import { useMemo } from "react";
import { CalendarDays, FileSpreadsheet, Shapes, Video } from "lucide-react";
import type { CalendarMeeting, EntryView, IdleSpan, Segment, Tag, WindowSpan, WorkBlock } from "../../api";
import { formatDuration } from "../../api";
import { addDays, formatTime, fromWallMs } from "../../lib/dates";
import { UNCATEGORIZED, tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import {
  HOUR_PX,
  HOUR_MS,
  MIN,
  FULL_LABEL_PX,
  LABEL_MIN_PX,
  type ColorLens,
  hourRange,
  topFn,
  HourRail,
  Column,
  NowLine,
} from "./grid";
import { Block, IdleBlock } from "./Block";
import { categoryBuckets } from "./buckets";
import { type SheetSpan, sheetLines, sheetLinesWidth, TimesheetLines } from "./TimesheetLines";

/** Tıklanan anı çevreleyen boşluk: 2 saate kadarsa tamamı, değilse tıklanan çeyrekten 1 saat. */
export function gapAround(
  t: number,
  spans: { start: string; end: string }[],
  dayStart: number,
): [number, number] | null {
  let prevEnd = dayStart;
  let nextStart = Math.min(+addDays(new Date(dayStart), 1), Date.now());
  for (const s of spans) {
    const a = +new Date(s.start);
    const b = +new Date(s.end);
    if (a <= t && t < b) return null;
    if (b <= t) prevEnd = Math.max(prevEnd, b);
    if (a > t) nextStart = Math.min(nextStart, a);
  }
  if (t >= nextStart) return null;
  let start = prevEnd;
  let end = nextStart;
  if (end - start > 2 * HOUR_MS) {
    start = Math.max(prevEnd, Math.floor(t / (15 * MIN)) * 15 * MIN);
    end = Math.min(nextStart, start + HOUR_MS);
  }
  return end - start >= 5 * MIN ? [start, end] : null;
}

/** Eklenmek üzere seçilen aralık: kesik çizgili hayalet blok. */
export function Preview({ range, top }: { range?: [number, number] | null; top: (t: number) => number }) {
  if (!range) return null;
  const t = top(range[0]);
  return (
    <span
      className="pointer-events-none absolute inset-x-0.5 z-10 grid place-items-center rounded-[5px] border-2 border-dashed border-primary/60 bg-primary/10 text-[11px] font-medium text-primary"
      style={{ top: t, height: Math.max(14, top(range[1]) - t - 2) }}
    >
      {formatTime(new Date(range[0]))} – {formatTime(new Date(range[1]))}
    </span>
  );
}

function blockGeometry(b: { start: string; end: string }, top: (t: number) => number) {
  const t = top(+new Date(b.start));
  return { top: t, height: Math.max(3, top(+new Date(b.end)) - t - 2) };
}

/** Takvimin dilimi: bloklar en az bu süre kadar yüksek çizilir ki rahat tıklansın. */
const SLOT_MS = 15 * MIN;
/** Bundan kısa oturum ve boşta süre takvimde boş kalır (dilimin üçte biri). */
const SHOW_MIN_MS = SLOT_MS / 3;

type SessionItem = { start: string; end: string; ms: number } & (
  { block: WorkBlock; idle?: undefined } | { idle: IdleSpan; block?: undefined }
);

/**
 * Oturumlar sütununun yerleşimi: kısa (anlamsız) bloklar gösterilmez; kalanlar en az bir
 * dilim yüksekliğinde, ama bir sonrakinin üstüne binmeden çizilir.
 */
export function placeSessions(
  blocks: WorkBlock[],
  idle: IdleSpan[],
  top: (t: number) => number,
  hourPx: number,
): (SessionItem & { top: number; height: number })[] {
  const items: SessionItem[] = [
    ...blocks.map((b) => ({ start: b.start, end: b.end, ms: b.activeSeconds * 1000, block: b })),
    ...idle.map((s) => ({ start: s.start, end: s.end, ms: +new Date(s.end) - +new Date(s.start), idle: s })),
  ];
  const shown = items.filter((i) => i.ms >= SHOW_MIN_MS).sort((a, b) => +new Date(a.start) - +new Date(b.start));
  const minHeight = (SLOT_MS / HOUR_MS) * hourPx - 2;
  return shown.map((item, k) => {
    const g = blockGeometry(item, top);
    const next = shown[k + 1];
    const room = next ? top(+new Date(next.start)) - g.top - 2 : Infinity;
    return { ...item, top: g.top, height: Math.max(g.height, Math.min(minHeight, room)) };
  });
}

/**
 * Gün takvimi: Oturumlar · Zaman çizelgesi satırları (çizelge kuruluysa) · Toplantılar (takvim
 * bağlıysa) · Kategori şeridi.
 */
export function DayCalendar({
  from,
  blocks,
  idle = [],
  segments,
  windows,
  tags,
  meetings = null,
  entries = null,
  onMeeting,
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
  lens = "category",
  sheet = [],
}: {
  from: Date;
  blocks: WorkBlock[];
  /** Bilgisayardan uzakta geçen, atanmamış süre. */
  idle?: IdleSpan[];
  segments: Segment[];
  /** Pencere aralıkları: bloğa tıklayınca hangi pencerelerde zaman geçtiği görünür. */
  windows?: WindowSpan[];
  tags: Map<string, Tag>;
  /** Takvim toplantıları; `null`: takvim bağlı değil (sütun gösterilmez). */
  meetings?: CalendarMeeting[] | null;
  /** Günün zaman çizelgesi satırları (bütün çizelgeler); `null`: çizelge yok (sütun gösterilmez). */
  entries?: EntryView[] | null;
  /** Toplantıya tıklanınca (projeye atama menüsü) ve tıklanan nokta. */
  onMeeting?: (m: CalendarMeeting, x: number, y: number) => void;
  onEmpty?: (start: number, end: number) => void;
  /** Sürükleyerek seçilen aralık (zaman damgası) ve bırakılan nokta. */
  onRange?: (start: number, end: number, x: number, y: number) => void;
  preview?: [number, number] | null;
  /** Bir saatin yüksekliği (yakınlaştırma). */
  hourPx?: number;
  /** Bloklar kategori ya da proje renginde. */
  lens?: ColorLens;
  /** Zaman çizelgesi satırlarının aralıkları: çizelge merceğinde blokların başlığı ve rengi. */
  sheet?: SheetSpan[];
}) {
  // Gece yarısını aşan toplantılar güne kırpılır: ızgaranın dışına taşmasınlar, saat
  // aralığı da günün içindeki kısmına göre genişlesin.
  const meetingSpans = useMemo(() => {
    const dayStart = +from;
    const dayEnd = +new Date(from.getFullYear(), from.getMonth(), from.getDate() + 1);
    return (meetings ?? []).flatMap((m) => {
      const start = Math.max(+new Date(m.start), dayStart);
      const end = Math.min(+new Date(m.end), dayEnd);
      return end > start ? [{ m, start: new Date(start).toISOString(), end: new Date(end).toISOString() }] : [];
    });
  }, [meetings, from]);
  const lines = useMemo(() => (entries ? sheetLines(entries, from) : []), [entries, from]);
  const range = useMemo(() => {
    const lineSpans = lines.map((l) => ({
      start: new Date(l.start).toISOString(),
      end: new Date(l.end).toISOString(),
    }));
    return hourRange(from, [...blocks, ...idle, ...segments, ...meetingSpans, ...lineSpans], hourPx);
  }, [from, blocks, idle, segments, meetingSpans, lines, hourPx]);
  const top = topFn(+from, range);
  // 5 dk'lık dilim ancak rahat tıklanacak kadar yüksekse; yoksa 15 dk.
  const step = range.px >= 240 ? 5 : 15;
  const buckets = useMemo(() => categoryBuckets(segments, +from, step), [segments, from, step]);
  const showMeetings = meetings !== null;
  const showSheet = entries !== null;
  // Sütunlar: saatler, oturumlar, [çizelge], [toplantılar], kategori şeridi.
  const columns = [
    "40px",
    "minmax(0,1fr)",
    ...(showSheet ? [`${sheetLinesWidth(lines)}px`] : []),
    ...(showMeetings ? ["minmax(0,0.6fr)"] : []),
    "14px",
  ];
  const grid = { gridTemplateColumns: columns.join(" ") };
  const strip = columns.length;

  return (
    <div>
      <div
        className="sticky top-(--cal-head) z-20 grid gap-x-2 bg-card pb-2 text-[11px] font-medium text-muted-foreground"
        style={grid}
      >
        <span />
        <span>Oturumlar</span>
        {showSheet && (
          <span className="flex justify-center" title="Zaman çizelgesi satırları">
            <FileSpreadsheet className="size-3" />
          </span>
        )}
        {showMeetings && <span>Toplantılar</span>}
        <span title="Kategori: her aralıkta en çok süren">
          <Shapes className="size-3" />
        </span>
      </div>
      <div className="grid gap-x-2 pt-1.5" style={grid}>
        <div className="col-start-1 row-start-1">
          <HourRail range={range} />
        </div>
        <Column
          range={range}
          className="col-start-2 row-start-1"
          onEmpty={
            onEmpty &&
            ((offset) => {
              const gap = gapAround(fromWallMs(offset, +from), segments, +from);
              if (gap) onEmpty(...gap);
            })
          }
          onRange={onRange && ((a, b, x, y) => onRange(fromWallMs(a, +from), fromWallMs(b, +from), x, y))}
        >
          {placeSessions(blocks, idle, top, range.px).map(({ block, idle: span, top: t, height }) =>
            span ? (
              <IdleBlock key={`idle-${span.start}`} span={span} onSelect={onRange} top={t} height={height} />
            ) : (
              <Block
                key={block.start}
                b={block}
                tags={tags}
                lens={lens}
                sheet={sheet}
                windows={windows}
                top={t}
                height={height}
                hourPx={range.px}
                dayStart={+from}
              />
            ),
          )}
          <Preview range={preview} top={top} />
          <NowLine day={from} range={range} />
        </Column>
        {showSheet && (
          <div
            className="relative col-start-3 row-start-1 rounded-sm bg-muted/40"
            style={{ height: (range.last - range.first) * range.px }}
          >
            <TimesheetLines lines={lines} tags={tags} top={top} />
          </div>
        )}
        {showMeetings && (
          <Column range={range} className={cn("row-start-1", showSheet ? "col-start-4" : "col-start-3")}>
            {meetingSpans.map(({ m, ...span }, i) => (
              <MeetingBlock
                key={`${m.uid}-${m.start}-${i}`}
                m={m}
                project={m.projectId ? tags.get(m.projectId) : undefined}
                onClick={onMeeting}
                {...blockGeometry(span, top)}
              />
            ))}
            <NowLine day={from} range={range} />
          </Column>
        )}
        <div
          className="relative row-start-1"
          style={{ gridColumnStart: strip, height: (range.last - range.first) * range.px }}
        >
          {buckets.map((b) => {
            const t = top(b.start);
            const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
            const tip = [
              `${formatTime(new Date(b.start))}–${formatTime(new Date(b.end))}`,
              ...b.shares.map(
                (c) => `${(c.id ? tags.get(c.id)?.name : null) ?? UNCATEGORIZED} %${Math.round(c.share * 100)}`,
              ),
            ].join("\n");
            return (
              <button
                key={b.start}
                type="button"
                disabled={!onRange}
                className="absolute inset-x-0 rounded-[2px] enabled:cursor-pointer enabled:hover:brightness-90 dark:enabled:hover:brightness-125"
                style={{
                  top: t,
                  height: Math.max(2, top(b.end) - t - 1),
                  background: tagColor(tag),
                  // Aralığın az kısmı takip edildiyse soluk.
                  opacity: 0.35 + 0.65 * b.coverage,
                }}
                title={onRange ? `${tip}\nTıkla: kategoriye ya da projeye ata` : tip}
                aria-label={tip}
                onClick={(e) => onRange?.(b.start, b.end, e.clientX, e.clientY)}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** Takvim toplantısı; projesi varsa proje renginde. Tıklayınca projeye atama menüsü. */
function MeetingBlock({
  m,
  project,
  top,
  height,
  onClick,
}: {
  m: CalendarMeeting;
  project?: Tag;
  top: number;
  height: number;
  onClick?: (m: CalendarMeeting, x: number, y: number) => void;
}) {
  const a = new Date(m.start);
  const b = new Date(m.end);
  const time = `${formatTime(a)}–${formatTime(b)}`;
  const tip = [
    m.subject || "(konusuz)",
    `${time} · ${formatDuration((+b - +a) / 1000)}`,
    m.location,
    project && `Proje: ${project.name}`,
    m.agenda && `\n${m.agenda}`,
  ]
    .filter(Boolean)
    .join("\n");
  const Icon = m.online ? Video : CalendarDays;
  const color = project ? tagColor(project) : undefined;
  return (
    <button
      type="button"
      disabled={!onClick}
      className={cn(
        "absolute inset-x-0.5 overflow-hidden rounded-[5px] border px-1.5 text-left enabled:cursor-pointer",
        project
          ? "border-l-[3px] text-foreground enabled:hover:brightness-95 dark:enabled:hover:brightness-125"
          : "border-dashed border-primary/50 bg-primary/8 text-primary enabled:hover:bg-primary/15",
        m.ignored && "opacity-50",
      )}
      style={
        color
          ? {
              top,
              height,
              borderColor: `color-mix(in srgb, ${color} 45%, transparent)`,
              borderLeftColor: color,
              background: `color-mix(in srgb, ${color} 14%, var(--card))`,
            }
          : { top, height }
      }
      title={onClick ? `${tip}\nTıkla: projeye ata ya da kayıt ekle` : tip}
      aria-label={tip}
      onClick={(e) => {
        // Klavyeyle basılınca imleç konumu yok: menü bloğun yanında açılır.
        const r = e.currentTarget.getBoundingClientRect();
        const [x, y] = e.detail === 0 ? [r.right, r.top] : [e.clientX, e.clientY];
        onClick?.(m, x, y);
      }}
    >
      {height >= LABEL_MIN_PX && (
        <span className={cn("flex h-full flex-col", height >= FULL_LABEL_PX ? "py-1" : "justify-center")}>
          <span className="flex items-center gap-1">
            <Icon className="size-3 shrink-0" style={color ? { color } : undefined} />
            <span className="truncate text-[11px] leading-tight font-semibold">{m.subject || "(konusuz)"}</span>
          </span>
          {height >= FULL_LABEL_PX && (
            <span
              className={cn(
                "truncate text-[10px] leading-tight tabular",
                project ? "text-muted-foreground" : "text-primary/75",
              )}
            >
              {project ? `${project.name} · ${time}` : time}
              {m.location && ` · ${m.location}`}
            </span>
          )}
        </span>
      )}
    </button>
  );
}
