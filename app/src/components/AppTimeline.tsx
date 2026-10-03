import { useMemo, useState } from "react";
import { ChevronRight, PenLine } from "lucide-react";
import { formatDuration, type Tag, type WindowSpan } from "../api";
import { formatTime, today, wallMs } from "../lib/dates";
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

/** Gösterilecek saat aralığı: etkinliğe göre, en az 08–18. */
function hourRange(from: Date, windows: WindowSpan[]) {
  let first = 8;
  let last = 18;
  for (const w of windows) {
    const a = wallMs(+new Date(w.start), +from) / HOUR_MS;
    const b = wallMs(+new Date(w.end), +from) / HOUR_MS;
    first = Math.min(first, Math.floor(Math.max(0, a)));
    last = Math.max(last, Math.ceil(Math.min(24, b)));
  }
  return { first, last };
}

type Hover = { span: WindowSpan; x: number; y: number };

/**
 * Uygulama çizelgesi: her uygulama bir şerit, kullanıldığı saatler çubuk.
 * Satıra tıklayınca pencere başlıkları ayrı şeritlerde açılır.
 */
export default function AppTimeline({
  from,
  windows,
  tags,
}: {
  from: Date;
  windows: WindowSpan[];
  tags: Map<string, Tag>;
}) {
  const { apps, rest } = useMemo(() => group(windows), [windows]);
  const range = useMemo(() => hourRange(from, windows), [from, windows]);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [hover, setHover] = useState<Hover | null>(null);

  // Konumlar duvar saatine göre: yaz saati geçişinde de saat etiketleriyle hizalı.
  const startMs = range.first * HOUR_MS;
  const spanMs = (range.last - range.first) * HOUR_MS;
  const frac = (t: number) => (wallMs(t, +from) - startMs) / spanMs;
  const pos = (w: WindowSpan) => {
    const a = Math.max(0, frac(+new Date(w.start)));
    const b = Math.min(1, frac(+new Date(w.end)));
    return { left: `${a * 100}%`, width: `max(2px, ${(b - a) * 100}%)` };
  };
  const hours = Array.from({ length: range.last - range.first + 1 }, (_, i) => range.first + i);
  const nowFrac = +from === +today() ? frac(Date.now()) : -1;

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
    lane.spans.map((w, i) => (
      <span
        key={i}
        className={cn("absolute inset-y-1 rounded-[3px]", muted && "opacity-70")}
        style={{ ...pos(w), background: spanColor(w) }}
        onMouseEnter={(e) => setHover({ span: w, x: e.clientX, y: e.clientY })}
        onMouseMove={(e) => setHover({ span: w, x: e.clientX, y: e.clientY })}
        onMouseLeave={() => setHover(null)}
      />
    ));

  const grid = (
    <>
      {hours.map((h) => (
        <span
          key={h}
          className="absolute inset-y-0 border-l border-border/60"
          style={{ left: `${((h - range.first) / (range.last - range.first)) * 100}%` }}
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
