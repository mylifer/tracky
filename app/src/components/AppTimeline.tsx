import { useMemo, useState } from "react";
import { ChevronRight, PenLine } from "lucide-react";
import { formatDuration, type Tag, type WindowSpan } from "../api";
import { addDays, formatTime, isoDate, today, wallMs } from "../lib/dates";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

const HOUR_MS = 3600_000;
/** Bir uygulama açıldığında en çok bu kadar pencere ayrı satırda; kalanı "diğer". */
const MAX_TITLES = 8;
/** Bundan kısa süren uygulamalar "Diğer uygulamalar" satırında toplanır. */
const MIN_APP_SECS = 60;

type Lane = { key: string; label: string; secs: number; spans: WindowSpan[] };
type AppLane = Lane & { appId: string; categoryId: string | null; titles: Lane[] };

const secsOf = (spans: WindowSpan[]) => spans.reduce((s, w) => s + (+new Date(w.end) - +new Date(w.start)) / 1000, 0);

function group(windows: WindowSpan[]): { apps: AppLane[]; rest: Lane | null } {
  const byApp = new Map<string, WindowSpan[]>();
  for (const w of windows) byApp.set(w.appId, [...(byApp.get(w.appId) ?? []), w]);
  const lanes: AppLane[] = [...byApp.entries()].map(([appId, spans]) => {
    const byTitle = new Map<string, WindowSpan[]>();
    for (const w of spans) byTitle.set(w.title, [...(byTitle.get(w.title) ?? []), w]);
    const titles = [...byTitle.entries()]
      .map(([title, s]) => ({ key: title, label: title || "(başlıksız)", secs: secsOf(s), spans: s }))
      .sort((a, b) => b.secs - a.secs);
    if (titles.length > MAX_TITLES) {
      const extra = titles.splice(MAX_TITLES - 1);
      const s = extra.flatMap((t) => t.spans);
      titles.push({ key: "\u0000rest", label: `Diğer ${extra.length} pencere`, secs: secsOf(s), spans: s });
    }
    // Kategori: en çok sürenin kategorisi (elle atama pencere bazında farklı olabilir).
    const cat = new Map<string | null, number>();
    for (const w of spans) cat.set(w.categoryId, (cat.get(w.categoryId) ?? 0) + +new Date(w.end) - +new Date(w.start));
    const categoryId = [...cat.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
    return { key: appId, appId, label: spans[0].appName, secs: secsOf(spans), spans, categoryId, titles };
  });
  lanes.sort((a, b) => b.secs - a.secs);
  const apps = lanes.filter((l) => l.secs >= MIN_APP_SECS);
  const small = lanes.filter((l) => l.secs < MIN_APP_SECS);
  const s = small.flatMap((l) => l.spans);
  return {
    apps,
    rest: small.length
      ? { key: "\u0000rest", label: `Diğer ${small.length} uygulama`, secs: secsOf(s), spans: s }
      : null,
  };
}

/** Bir pencere aralığının bir güne düşen parçası: günün duvar saatiyle (ms). */
type Piece = { day: number; a: number; b: number };

/** Aralığı gün sınırlarından böler; `starts` gün başları + son günün bitişi. */
function pieces(w: WindowSpan, starts: number[]): Piece[] {
  const t0 = +new Date(w.start);
  const t1 = +new Date(w.end);
  const out: Piece[] = [];
  for (let d = 0; d < starts.length - 1; d++) {
    const ds = starts[d];
    const de = starts[d + 1];
    if (t1 <= ds || t0 >= de) continue;
    out.push({ day: d, a: wallMs(Math.max(t0, ds), ds), b: wallMs(Math.min(t1, de), ds) });
  }
  return out;
}

/** Gösterilecek saat aralığı (her gün için aynı): etkinliğe göre, en az 08–18. */
function hourRange(windows: WindowSpan[], starts: number[]) {
  let first = 8;
  let last = 18;
  for (const w of windows) {
    for (const p of pieces(w, starts)) {
      first = Math.min(first, Math.floor(Math.max(0, p.a / HOUR_MS)));
      last = Math.max(last, Math.ceil(Math.min(24, p.b / HOUR_MS)));
    }
  }
  return { first, last };
}

type Hover = { span: WindowSpan; x: number; y: number };

/**
 * Uygulama çizelgesi: her uygulama bir şerit, kullanıldığı saatler çubuk.
 * Satıra tıklayınca pencere başlıkları ayrı şeritlerde açılır. Birden çok günde
 * yatay eksen günlere bölünür; her günün diliminde aynı saat aralığı gösterilir.
 */
export default function AppTimeline({
  from,
  days = 1,
  windows,
  tags,
  onSelectDay,
}: {
  from: Date;
  days?: number;
  windows: WindowSpan[];
  tags: Map<string, Tag>;
  onSelectDay?: (iso: string) => void;
}) {
  const dates = useMemo(() => Array.from({ length: days + 1 }, (_, i) => addDays(from, i)), [from, days]);
  const starts = useMemo(() => dates.map(Number), [dates]);
  const { apps, rest } = useMemo(() => group(windows), [windows]);
  const range = useMemo(() => hourRange(windows, starts), [windows, starts]);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [hover, setHover] = useState<Hover | null>(null);

  // Konumlar duvar saatine göre: yaz saati geçişinde de saat etiketleriyle hizalı.
  const startMs = range.first * HOUR_MS;
  const spanMs = (range.last - range.first) * HOUR_MS;
  const clamp = (x: number) => Math.min(1, Math.max(0, x));
  /** Günün `day` dilimindeki duvar saatinin (ms) şerit üzerindeki yeri (0–1). */
  const x = (day: number, wall: number) => (day + clamp((wall - startMs) / spanMs)) / days;
  const hours = Array.from({ length: range.last - range.first + 1 }, (_, i) => range.first + i);
  const todayIndex = starts.indexOf(+today());
  // Şimdi çizgisi: bugün gösteriliyorsa ve saat aralığın içindeyse.
  const nowWall = todayIndex >= 0 && todayIndex < days ? wallMs(Date.now(), starts[todayIndex]) : NaN;
  const nowFrac = nowWall >= startMs && nowWall <= startMs + spanMs ? x(todayIndex, nowWall) : -1;
  const weekday = new Intl.DateTimeFormat("tr-TR", { weekday: "short" });

  function toggle(key: string) {
    setOpen((s) => {
      const next = new Set(s);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  // Her çubuk kendi penceresinin kategorisinde (örn. Chrome'da GitHub ile YouTube farklı renk).
  const spanColor = (w: WindowSpan) => tagColor(w.categoryId ? tags.get(w.categoryId) : undefined);
  const bars = (lane: Lane, muted = false) =>
    lane.spans.flatMap((w, i) =>
      pieces(w, starts).map((p) => {
        const a = x(p.day, p.a);
        const b = x(p.day, p.b);
        return (
          <span
            key={`${i}:${p.day}`}
            className={cn("absolute inset-y-1 rounded-[3px]", muted && "opacity-70")}
            style={{ left: `${a * 100}%`, width: `max(2px, ${(b - a) * 100}%)`, background: spanColor(w) }}
            onMouseEnter={(e) => setHover({ span: w, x: e.clientX, y: e.clientY })}
            onMouseMove={(e) => setHover({ span: w, x: e.clientX, y: e.clientY })}
            onMouseLeave={() => setHover(null)}
          />
        );
      }),
    );

  const grid = (
    <>
      {days === 1
        ? hours.map((h) => (
            <span
              key={h}
              className="absolute inset-y-0 border-l border-border/60"
              style={{ left: `${((h - range.first) / (range.last - range.first)) * 100}%` }}
            />
          ))
        : dates
            .slice(0, days)
            .map((_, i) => (
              <span
                key={i}
                className={cn("absolute inset-y-0 border-l border-border", i === todayIndex && "bg-primary/[0.04]")}
                style={{ left: `${(i / days) * 100}%`, width: `${100 / days}%` }}
              />
            ))}
      {nowFrac >= 0 && nowFrac <= 1 && (
        <span className="absolute inset-y-0 z-10 w-px bg-destructive" style={{ left: `${nowFrac * 100}%` }} />
      )}
    </>
  );

  const row = "grid grid-cols-[minmax(0,190px)_minmax(0,1fr)] items-center gap-3";

  return (
    <div className="relative" onMouseLeave={() => setHover(null)}>
      <div className={cn(row, "pb-1.5 text-[10px] text-muted-foreground tabular")}>
        <span className="text-[11px] font-medium">Uygulama</span>
        {days === 1 ? (
          <div className="relative h-4">
            {hours.map((h) => (
              <span
                key={h}
                className="absolute -translate-x-1/2"
                style={{ left: `${((h - range.first) / (range.last - range.first)) * 100}%` }}
              >
                {String(h % 24).padStart(2, "0")}
              </span>
            ))}
          </div>
        ) : (
          <div className="flex" title={`Her gün ${pad(range.first)}:00–${pad(range.last)}:00`}>
            {dates.slice(0, days).map((d, i) => (
              <button
                key={i}
                className={cn(
                  "min-w-0 flex-1 truncate rounded-sm py-0.5 text-[11px] hover:bg-accent",
                  i === todayIndex ? "font-semibold text-foreground" : "text-muted-foreground",
                )}
                onClick={() => onSelectDay?.(isoDate(d))}
              >
                {weekday.format(d)} {d.getDate()}
              </button>
            ))}
          </div>
        )}
      </div>

      <ul className="divide-y divide-border/60">
        {apps.map((app) => {
          const color = tagColor(app.categoryId ? tags.get(app.categoryId) : undefined);
          const expanded = open.has(app.key);
          const manual = app.appId.startsWith("kum.manual/");
          return (
            <li key={app.key}>
              <button
                className={cn(row, "w-full py-0.5 text-left hover:bg-accent/50")}
                onClick={() => toggle(app.key)}
                aria-expanded={expanded}
              >
                <span className="flex min-w-0 items-center gap-1.5 text-xs">
                  <ChevronRight
                    className={cn(
                      "size-3 shrink-0 text-muted-foreground transition-transform",
                      expanded && "rotate-90",
                    )}
                  />
                  {manual ? (
                    <PenLine className="size-3 shrink-0 text-muted-foreground" />
                  ) : (
                    <i className="size-2 shrink-0 rounded-full" style={{ background: color }} />
                  )}
                  <span className="min-w-0 flex-1 truncate font-medium">{app.label}</span>
                  <span className="shrink-0 text-[11px] text-muted-foreground tabular">{formatDuration(app.secs)}</span>
                </span>
                <span className="relative block h-7">
                  {grid}
                  {bars(app)}
                </span>
              </button>
              {expanded && (
                <ul className="pb-1">
                  {app.titles.map((t) => (
                    <li key={t.key} className={row}>
                      <span className="flex min-w-0 items-center gap-1.5 pl-[30px] text-[11px]" title={t.label}>
                        <span className="min-w-0 flex-1 truncate text-muted-foreground">{t.label}</span>
                        <span className="shrink-0 text-muted-foreground tabular">{formatDuration(t.secs)}</span>
                      </span>
                      <span className="relative block h-5">
                        {grid}
                        {bars(t, true)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
        {rest && (
          <li className={cn(row, "py-0.5")}>
            <span className="flex min-w-0 items-center gap-1.5 pl-[18px] text-xs text-muted-foreground">
              <span className="min-w-0 flex-1 truncate">{rest.label}</span>
              <span className="shrink-0 text-[11px] tabular">{formatDuration(rest.secs)}</span>
            </span>
            <span className="relative block h-6">
              {grid}
              {bars(rest, true)}
            </span>
          </li>
        )}
      </ul>

      {hover && <HoverCard hover={hover} tags={tags} />}
    </div>
  );
}

const pad = (h: number) => String(h % 24).padStart(2, "0");

function HoverCard({ hover, tags }: { hover: Hover; tags: Map<string, Tag> }) {
  const w = hover.span;
  const tag = w.categoryId ? tags.get(w.categoryId) : undefined;
  const secs = (+new Date(w.end) - +new Date(w.start)) / 1000;
  // İmlecin sağında; ekranın sağına taşacaksa solunda.
  const left = hover.x + 260 > window.innerWidth ? hover.x - 252 : hover.x + 12;
  return (
    <div
      className="pointer-events-none fixed z-50 w-60 rounded-lg border bg-popover px-3 py-2 text-popover-foreground shadow-lg"
      style={{ left, top: hover.y + 14 }}
    >
      <div className="flex items-center gap-1.5 text-xs font-medium">
        <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
        <span className="min-w-0 truncate">{w.appName}</span>
      </div>
      {w.title && <p className="mt-0.5 line-clamp-2 text-[11px] break-words text-muted-foreground">{w.title}</p>}
      <p className="mt-1 text-[11px] tabular">
        {formatTime(new Date(w.start))} – {formatTime(new Date(w.end))} · {formatDuration(secs)}
        {tag && <span className="text-muted-foreground"> · {tag.name}</span>}
      </p>
    </div>
  );
}
