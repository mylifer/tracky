import { useMemo, useRef, useState } from "react";
import { Briefcase, CalendarDays, Shapes, Video, Zap } from "lucide-react";
import type { FocusTimer, Meeting, Segment, Tag, WorkBlock } from "../api";
import { formatDuration } from "../api";
import { addDays, formatTime, fromWallMs, isoDate, today, wallMs } from "../lib/dates";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";
import { BlockActions, useEdit } from "./SessionEdit";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

/** Yakınlaştırılmamış takvimde bir saatin yüksekliği. */
export const HOUR_PX = 52;
const HOUR_MS = 3600_000;
/** Bu yükseklikten küçük bloklarda yalnızca başlık gösterilir. */
const FULL_LABEL_PX = 40;
/** Bundan alçak bloklarda yazı yok (yalnızca renk; ayrıntı ipucunda). */
const LABEL_MIN_PX = 15;

/** Gösterilen saatler ve bir saatin piksel yüksekliği (yakınlaştırmayla değişir). */
type Range = { first: number; last: number; px: number };

/** Etkinliğe göre gösterilecek saat aralığı (en az 08–18). */
function hourRange(from: Date, spans: { start: string; end: string }[], px: number, days = 1): Range {
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
  return { first, last: Math.min(24, Math.max(last, first + 6)), px };
}

function topFn(dayStart: number, range: Range) {
  return (t: number) => ((wallMs(t, dayStart) - range.first * HOUR_MS) / HOUR_MS) * range.px;
}

function hours(range: Range) {
  const out = [];
  for (let h = range.first; h <= range.last; h++) out.push(h);
  return out;
}

/**
 * Saat aralarındaki çizgi ve etiketler (dakika): varsayılan görünümde yarım saat, yakınlaştıkça
 * çeyrek saat. Çok uzaklaştırınca (yarım saat birkaç piksele inince) gizlenir.
 */
export function subMarks(px: number, forLabels: boolean): number[] {
  const step = forLabels ? (px >= 300 ? 15 : px >= 40 ? 30 : 0) : px >= 200 ? 15 : px >= 24 ? 30 : 0;
  return step ? Array.from({ length: 60 / step - 1 }, (_, i) => (i + 1) * step) : [];
}

function HourRail({ range }: { range: Range }) {
  const marks = subMarks(range.px, true);
  return (
    // Yakınlaştırmada kaydırma bu ızgaranın üst kenarına göre sabitlenir.
    <div data-zoom-grid className="relative" style={{ height: (range.last - range.first) * range.px }}>
      {hours(range).map((h) => (
        <span key={h}>
          <span
            className="absolute right-1 -translate-y-1/2 text-[10px] text-muted-foreground tabular"
            style={{ top: (h - range.first) * range.px }}
          >
            {String(h % 24).padStart(2, "0")}:00
          </span>
          {h < range.last &&
            marks.map((m) => (
              <span
                key={m}
                className="absolute right-1 -translate-y-1/2 text-[9px] text-muted-foreground/70 tabular"
                style={{ top: (h - range.first + m / 60) * range.px }}
              >
                :{m}
              </span>
            ))}
        </span>
      ))}
    </div>
  );
}

/** Sürükleme 5 dakikaya yuvarlanır; bundan az kayma tıklama sayılır (piksel). */
const SNAP_MS = 5 * 60_000;
const DRAG_MIN_PX = 4;

function Column({
  range,
  children,
  className,
  onEmpty,
  onRange,
}: {
  range: Range;
  children: React.ReactNode;
  className?: string;
  /** Boş alana tıklanınca tıklanan duvar saatinin gece yarısından itibaren ms karşılığı. */
  onEmpty?: (offsetMs: number) => void;
  /** Boş alandan sürükleyerek seçilen aralık (duvar saati ms) ve bırakılan nokta. */
  onRange?: (fromMs: number, toMs: number, x: number, y: number) => void;
}) {
  const [drag, setDrag] = useState<{ a: number; b: number; y0: number } | null>(null);
  // Bloğun üzerinde basılınca hemen sürüklemeye geçilmez: kıpırdamadan bırakılırsa bloğa
  // tıklanmıştır (ayrıntı kartı açılır), kayarsa aralık seçimi başlar.
  const pending = useRef<{ a: number; y0: number } | null>(null);
  // Sürükleme bitince oluşan tıklama bloğun kartını açmasın.
  const swallowClick = useRef(false);
  const offsetAt = (el: HTMLElement, clientY: number) =>
    range.first * HOUR_MS + ((clientY - el.getBoundingClientRect().top) / range.px) * HOUR_MS;
  const snap = (ms: number) => Math.round(ms / SNAP_MS) * SNAP_MS;
  const interactive = !!(onEmpty || onRange);
  const ghost =
    drag && Math.abs(drag.b - drag.a) >= SNAP_MS ? [Math.min(drag.a, drag.b), Math.max(drag.a, drag.b)] : null;

  return (
    <div
      className={cn("relative touch-none select-none", interactive && "cursor-cell", className)}
      style={{ height: (range.last - range.first) * range.px }}
      title={interactive ? "Boş alana tıkla: elle kayıt · sürükle: aralığı seç" : undefined}
      onPointerDown={(e) => {
        // Bloğun ayrıntı kartı gibi portallardaki olaylar da React ağacında buraya yayılır;
        // yalnızca sütunun kendi alanında başlayan basışlar sürükleme başlatır.
        if (!interactive || e.button !== 0 || !e.currentTarget.contains(e.target as Node)) return;
        const at = snap(offsetAt(e.currentTarget, e.clientY));
        if ((e.target as HTMLElement).closest("button")) {
          if (onRange) pending.current = { a: at, y0: e.clientY };
          return;
        }
        e.currentTarget.setPointerCapture(e.pointerId);
        setDrag({ a: at, b: at, y0: e.clientY });
      }}
      onPointerMove={(e) => {
        const p = pending.current;
        if (p && Math.abs(e.clientY - p.y0) >= DRAG_MIN_PX) {
          pending.current = null;
          e.currentTarget.setPointerCapture(e.pointerId);
          setDrag({ a: p.a, b: snap(offsetAt(e.currentTarget, e.clientY)), y0: p.y0 });
        } else if (drag) {
          setDrag({ ...drag, b: snap(offsetAt(e.currentTarget, e.clientY)) });
        }
      }}
      onPointerUp={(e) => {
        pending.current = null;
        if (!drag) return;
        setDrag(null);
        if (Math.abs(e.clientY - drag.y0) < DRAG_MIN_PX) {
          onEmpty?.(offsetAt(e.currentTarget, e.clientY));
        } else {
          swallowClick.current = true;
          if (ghost) onRange?.(ghost[0], ghost[1], e.clientX, e.clientY);
        }
      }}
      onPointerCancel={() => {
        pending.current = null;
        setDrag(null);
      }}
      onClickCapture={(e) => {
        if (!swallowClick.current) return;
        swallowClick.current = false;
        e.stopPropagation();
        e.preventDefault();
      }}
    >
      {hours(range).map((h) => (
        <span key={h}>
          <span
            className="absolute inset-x-0 border-t border-border/70"
            style={{ top: (h - range.first) * range.px }}
          />
          {h < range.last &&
            subMarks(range.px, false).map((m) => (
              <span
                key={m}
                className="absolute inset-x-0 border-t border-dashed border-border/40"
                style={{ top: (h - range.first + m / 60) * range.px }}
              />
            ))}
        </span>
      ))}
      {children}
      {ghost && <DragGhost from={ghost[0]} to={ghost[1]} range={range} />}
    </div>
  );
}

/** Sürüklenen aralığın kesik çizgili önizlemesi (duvar saati ms). */
function DragGhost({ from, to, range }: { from: number; to: number; range: Range }) {
  const top = ((from - range.first * HOUR_MS) / HOUR_MS) * range.px;
  const label = (ms: number) => {
    const m = Math.round(ms / 60_000);
    return `${String(Math.floor(m / 60) % 24).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  };
  return (
    <span
      className="pointer-events-none absolute inset-x-0.5 z-20 grid place-items-center rounded-[5px] border-2 border-dashed border-primary/70 bg-primary/15 text-[11px] font-medium text-primary tabular"
      style={{ top, height: Math.max(14, ((to - from) / HOUR_MS) * range.px) }}
    >
      {label(from)} – {label(to)}
    </span>
  );
}

function NowLine({ day, range }: { day: Date; range: Range }) {
  if (+day !== +today()) return null;
  const top = topFn(+day, range)(Date.now());
  if (top < 0 || top > (range.last - range.first) * range.px) return null;
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
  const project = block.projectId ? tags.get(block.projectId) : undefined;
  const color = tagColor(tag);
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="outline" className="gap-1.5">
          <i className="size-2 rounded-full" style={{ background: color }} />
          {tag?.name ?? UNCATEGORIZED}
        </Badge>
        {project && (
          <Badge variant="outline" className="min-w-0 gap-1.5" title="Proje">
            <Briefcase className="shrink-0" style={{ color: tagColor(project) }} />
            <span className="truncate">{project.name}</span>
          </Badge>
        )}
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
          projectId={block.projectId}
          categories={edit.categories}
          projects={edit.projects}
          onChanged={edit.onChanged}
        />
      )}
    </div>
  );
}

function blockTitle(b: WorkBlock, tags: Map<string, Tag>) {
  const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
  const project = b.projectId ? tags.get(b.projectId) : undefined;
  return {
    tag,
    // Projeye atanmış blok proje adıyla görünür: atamanın sonucu takvimde hemen fark edilsin.
    title: project?.name ?? tag?.name ?? b.topApps[0]?.appName ?? UNCATEGORIZED,
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

/** Odak zamanlayıcısı aralıkları: blokların arkasında hafif mor şerit. */
function FocusBands({
  timers,
  top,
  className = "inset-x-0",
}: {
  timers: FocusTimer[];
  top: (t: number) => number;
  className?: string;
}) {
  return (
    <>
      {timers.map((f) => {
        const end = f.end ?? (Date.now() < +new Date(f.plannedEnd) ? new Date().toISOString() : f.plannedEnd);
        const t = top(+new Date(f.start));
        const h = Math.max(4, top(+new Date(end)) - t);
        const mins = Math.round((+new Date(end) - +new Date(f.start)) / 60000);
        return (
          <span
            key={f.id}
            className={cn("pointer-events-none absolute rounded-md bg-focus/12 ring-1 ring-focus/40", className)}
            style={{ top: t - 1, height: h + 2 }}
            title={`Odak zamanlayıcısı · ${mins} dk`}
          />
        );
      })}
    </>
  );
}

const MIN = 60_000;
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
function Preview({ range, top }: { range?: [number, number] | null; top: (t: number) => number }) {
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

/** Gün takvimi: Oturumlar · Toplantılar (takvim bağlıysa) · Kategori şeridi · Odak şeridi. */
export function DayCalendar({
  from,
  blocks,
  segments,
  tags,
  timers = [],
  meetings = null,
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
}: {
  from: Date;
  blocks: WorkBlock[];
  segments: Segment[];
  tags: Map<string, Tag>;
  timers?: FocusTimer[];
  /** Takvim toplantıları; `null`: takvim bağlı değil (sütun gösterilmez). */
  meetings?: Meeting[] | null;
  onEmpty?: (start: number, end: number) => void;
  /** Sürükleyerek seçilen aralık (zaman damgası) ve bırakılan nokta. */
  onRange?: (start: number, end: number, x: number, y: number) => void;
  preview?: [number, number] | null;
  /** Bir saatin yüksekliği (yakınlaştırma). */
  hourPx?: number;
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
  const range = useMemo(
    () => hourRange(from, [...blocks, ...segments, ...meetingSpans], hourPx),
    [from, blocks, segments, meetingSpans, hourPx],
  );
  const top = topFn(+from, range);
  const step = range.px >= 150 ? 5 : 15;
  const buckets = useMemo(() => categoryBuckets(segments, +from, step), [segments, from, step]);
  const showMeetings = meetings !== null;
  const grid = cn(
    "grid gap-x-2",
    showMeetings
      ? "grid-cols-[40px_minmax(0,1fr)_minmax(0,0.6fr)_10px_8px]"
      : "grid-cols-[40px_minmax(0,1fr)_10px_8px]",
  );
  const strip = showMeetings ? "col-start-4" : "col-start-3";
  const focus = showMeetings ? "col-start-5" : "col-start-4";

  return (
    <div>
      <div
        className={cn(grid, "sticky top-(--cal-head) z-20 bg-card pb-2 text-[11px] font-medium text-muted-foreground")}
      >
        <span />
        <span>Oturumlar</span>
        {showMeetings && <span>Toplantılar</span>}
        <span title="Kategori: her aralıkta en çok süren">
          <Shapes className="size-3" />
        </span>
        <span title="Odak blokları">
          <Zap className="size-3 text-focus" />
        </span>
      </div>
      <div className={cn(grid, "pt-1.5")}>
        <div className="col-start-1 row-start-1">
          <HourRail range={range} />
        </div>
        {/* Odak zamanlayıcıları oturumların arkasında; bloklar arasındaki boşluklarda görünür. */}
        <div
          className="relative col-start-2 row-start-1 -mx-1"
          style={{ height: (range.last - range.first) * range.px }}
        >
          <FocusBands timers={timers} top={top} />
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
          {blocks.map((b) => (
            <Block key={b.start} b={b} tags={tags} {...blockGeometry(b, top)} />
          ))}
          <Preview range={preview} top={top} />
          <NowLine day={from} range={range} />
        </Column>
        {showMeetings && (
          <Column range={range} className="col-start-3 row-start-1">
            {meetingSpans.map(({ m, ...span }, i) => (
              <MeetingBlock key={`${m.uid}-${m.start}-${i}`} m={m} onRange={onRange} {...blockGeometry(span, top)} />
            ))}
            <NowLine day={from} range={range} />
          </Column>
        )}
        <div className={cn("relative row-start-1", strip)} style={{ height: (range.last - range.first) * range.px }}>
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
        <div className={cn("relative row-start-1", focus)} style={{ height: (range.last - range.first) * range.px }}>
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

/** Takvim toplantısı; tıklayınca aralığı için atama menüsü (kayıt ekle, projeye ata). */
function MeetingBlock({
  m,
  top,
  height,
  onRange,
}: {
  m: Meeting;
  top: number;
  height: number;
  onRange?: (start: number, end: number, x: number, y: number) => void;
}) {
  const a = new Date(m.start);
  const b = new Date(m.end);
  const time = `${formatTime(a)}–${formatTime(b)}`;
  const tip = [m.subject || "(konusuz)", `${time} · ${formatDuration((+b - +a) / 1000)}`, m.location]
    .filter(Boolean)
    .join("\n");
  const Icon = m.online ? Video : CalendarDays;
  return (
    <button
      type="button"
      disabled={!onRange}
      className="absolute inset-x-0.5 overflow-hidden rounded-[5px] border border-dashed border-primary/50 bg-primary/8 px-1.5 text-left text-primary enabled:cursor-pointer enabled:hover:bg-primary/15"
      style={{ top, height }}
      title={onRange ? `${tip}\nTıkla: bu aralığa kayıt ekle ya da projeye ata` : tip}
      aria-label={tip}
      onClick={(e) => onRange?.(+a, +b, e.clientX, e.clientY)}
    >
      {height >= LABEL_MIN_PX && (
        <span className={cn("flex h-full flex-col", height >= FULL_LABEL_PX ? "py-1" : "justify-center")}>
          <span className="flex items-center gap-1">
            <Icon className="size-3 shrink-0" />
            <span className="truncate text-[11px] leading-tight font-semibold">{m.subject || "(konusuz)"}</span>
          </span>
          {height >= FULL_LABEL_PX && (
            <span className="truncate text-[10px] leading-tight text-primary/75 tabular">
              {time}
              {m.location && ` · ${m.location}`}
            </span>
          )}
        </span>
      )}
    </button>
  );
}

/** Hafta takvimi: her gün bir sütun, bloklar kategori renginde. */
export function WeekCalendar({
  from,
  blocks,
  dayTotals,
  tags,
  onSelectDay,
  timers = [],
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
}: {
  from: Date;
  blocks: WorkBlock[];
  dayTotals: number[];
  tags: Map<string, Tag>;
  onSelectDay: (iso: string) => void;
  timers?: FocusTimer[];
  onEmpty?: (start: number, end: number) => void;
  onRange?: (start: number, end: number, x: number, y: number) => void;
  preview?: [number, number] | null;
  hourPx?: number;
}) {
  const range = useMemo(() => hourRange(from, blocks, hourPx, 7), [from, blocks, hourPx]);
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
              <FocusBands
                timers={timers.filter((f) => +new Date(f.start) >= dayStart && +new Date(f.start) < dayEnd)}
                top={top}
                className="-inset-x-0.5"
              />
              {blocks
                .filter((b) => +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd)
                .map((b) => (
                  <Block key={b.start} b={b} tags={tags} narrow {...blockGeometry(b, top)} />
                ))}
              <Preview range={preview && preview[0] >= dayStart && preview[0] < dayEnd ? preview : null} top={top} />
              <NowLine day={d} range={range} />
            </Column>
          );
        })}
      </div>
    </div>
  );
}

export type CategoryBucket = {
  start: number;
  end: number;
  /** En çok süren kategori. */
  categoryId: string | null;
  /** Aralığın takip edilen oranı (0–1). */
  coverage: number;
  /** Kategorilerin takip edilen süredeki payı, büyükten küçüğe. */
  shares: { id: string | null; share: number }[];
};

/** Bundan az takip edilen aralık şeritte boş kalır. */
const BUCKET_MIN_MS = 30_000;

/**
 * Günü `stepMin` dakikalık aralıklara böler (duvar saatiyle); her aralık o sürede en çok
 * süren kategoriyi alır. Takip edilmeyen aralıklar atlanır.
 */
export function categoryBuckets(segments: Segment[], dayStart: number, stepMin: number): CategoryBucket[] {
  const step = stepMin * MIN;
  const count = Math.ceil((24 * HOUR_MS) / step);
  const sums: Map<string | null, number>[] = Array.from({ length: count }, () => new Map());
  for (const s of segments) {
    const a = wallMs(+new Date(s.start), dayStart);
    const b = wallMs(+new Date(s.end), dayStart);
    for (let i = Math.max(0, Math.floor(a / step)); i < count && i * step < b; i++) {
      const ms = Math.min(b, (i + 1) * step) - Math.max(a, i * step);
      if (ms > 0) sums[i].set(s.categoryId, (sums[i].get(s.categoryId) ?? 0) + ms);
    }
  }
  const out: CategoryBucket[] = [];
  sums.forEach((m, i) => {
    const total = [...m.values()].reduce((x, y) => x + y, 0);
    if (total < BUCKET_MIN_MS) return;
    const shares = [...m.entries()].sort((x, y) => y[1] - x[1]).map(([id, ms]) => ({ id, share: ms / total }));
    out.push({
      start: fromWallMs(i * step, dayStart),
      end: fromWallMs((i + 1) * step, dayStart),
      categoryId: shares[0].id,
      coverage: Math.min(1, total / step),
      shares,
    });
  });
  return out;
}
