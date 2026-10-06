import { useMemo, useRef, useState } from "react";
import { ChevronRight, PenLine } from "lucide-react";
import { type EditScope, formatDuration, NO_PROJECT, type Tag, type WindowSpan } from "../api";
import { addDays, formatTime, fromWallMs, isoDate, today, wallMs } from "../lib/dates";
import { tagColor } from "../lib/tags";
import { detailRows, onlyOther, type MinorBond } from "../lib/minorWindows";
import { cn } from "../lib/utils";
import { clampZoom, useZoomGestures } from "../lib/zoom";
import { AppIcon } from "./AppIcon";

const HOUR_MS = 3600_000;
/** Bir uygulama açıldığında en çok bu kadar satır; kalanı "Diğer". */
const MAX_TITLES = 8;
/** Bundan kısa süren uygulamalar "Diğer uygulamalar" satırında toplanır. */
const MIN_APP_SECS = 60;
const MIN_MS = 60_000;
/** Dilim boyları (dk): çubuklar bu ızgaraya oturur, tek tek pencereler değil. */
const SLOT_STEPS = [5, 10, 15, 30, 60];

type Lane = { key: string; label: string; secs: number; spans: WindowSpan[] };
type AppLane = Lane & { appId: string; categoryId: string | null; titles: Lane[] };

const secsOf = (spans: WindowSpan[]) => spans.reduce((s, w) => s + (+new Date(w.end) - +new Date(w.start)) / 1000, 0);

/** Pencerenin (başlığın) bağı: süresinin çoğunun yazıldığı proje, yoksa sitesi. */
function bondOf(spans: WindowSpan[], tags: Map<string, Tag>): MinorBond | null {
  const ms = new Map<string | null, number>();
  for (const w of spans) {
    const id = w.projectId && w.projectId !== NO_PROJECT ? w.projectId : null;
    ms.set(id, (ms.get(id) ?? 0) + +new Date(w.end) - +new Date(w.start));
  }
  const top = [...ms.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
  const project = top ? tags.get(top) : undefined;
  if (project) return { key: `p:${project.id}`, label: project.name, projectId: project.id };
  const domain = spans.find((w) => w.domain)?.domain;
  return domain ? { key: `d:${domain}`, label: domain, domain } : null;
}

export function group(windows: WindowSpan[], tags: Map<string, Tag>): { apps: AppLane[]; rest: Lane | null } {
  const byApp = new Map<string, WindowSpan[]>();
  for (const w of windows) byApp.set(w.appId, [...(byApp.get(w.appId) ?? []), w]);
  const lanes: AppLane[] = [...byApp.entries()].map(([appId, spans]) => {
    const byTitle = new Map<string, WindowSpan[]>();
    for (const w of spans) byTitle.set(w.title, [...(byTitle.get(w.title) ?? []), w]);
    const each = [...byTitle.entries()]
      .map(([title, s]) => ({ key: title, label: title || "(başlıksız)", secs: secsOf(s), spans: s }))
      .sort((a, b) => b.secs - a.secs);
    // Kısa pencereler adıyla değil, ortak projede/sitede ya da "Diğer"de toplanır.
    const rows = detailRows(
      each,
      (t) => t.secs,
      (t) => bondOf(t.spans, tags),
      "pencere",
      MAX_TITLES,
    );
    const titles: Lane[] = onlyOther(rows)
      ? []
      : rows.map((r) => {
          if (r.kind === "item") return r.item;
          const s = r.items.flatMap((t) => t.spans);
          return { key: r.key, label: r.label, secs: r.seconds, spans: s };
        });
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

/**
 * Izgaraya oturmuş çubuk: art arda dolu dilimlerin birleşimi. `a`/`b` günün duvar saati,
 * `start`/`end` zaman damgası; `ms` içinde gerçekten geçen süre.
 */
export type SlotBar = {
  day: number;
  a: number;
  b: number;
  start: number;
  end: number;
  ms: number;
  categoryId: string | null;
  /** En çok süren pencereler (en fazla 3). */
  titles: { title: string; ms: number }[];
};

/**
 * Görünen aralığa göre dilim boyu: tam günde 15 dk, yakınlaştıkça incelir (en az 5 dk);
 * çok günde ise dilim günlerle orantılı büyür ki çubuklar tıklanabilir kalsın.
 */
export function slotMinutes(days: number, zoom: number): number {
  const want = (15 * days) / Math.max(1, zoom);
  return SLOT_STEPS.find((m) => m >= want - 1e-9) ?? SLOT_STEPS[SLOT_STEPS.length - 1];
}

/**
 * Pencereleri `slotMin` dakikalık dilimlere toplar; dilimin en az üçte biri doluysa dilim
 * dolu sayılır, değilse boş kalır. Art arda dolu dilimler tek çubuk olur.
 */
export function slotBars(spans: WindowSpan[], starts: number[], slotMin: number): SlotBar[] {
  const step = slotMin * MIN_MS;
  type Acc = { ms: number; cats: Map<string | null, number>; titles: Map<string, number> };
  const slots = new Map<string, Acc>();
  for (const w of spans) {
    for (const p of pieces(w, starts)) {
      for (let i = Math.floor(p.a / step); i * step < p.b; i++) {
        const ms = Math.min(p.b, (i + 1) * step) - Math.max(p.a, i * step);
        if (ms <= 0) continue;
        const key = `${p.day}:${i}`;
        const acc = slots.get(key) ?? { ms: 0, cats: new Map(), titles: new Map() };
        acc.ms += ms;
        acc.cats.set(w.categoryId, (acc.cats.get(w.categoryId) ?? 0) + ms);
        acc.titles.set(w.title, (acc.titles.get(w.title) ?? 0) + ms);
        slots.set(key, acc);
      }
    }
  }
  const filled = [...slots.entries()]
    .filter(([, acc]) => acc.ms >= step / 3)
    .map(([key, acc]) => {
      const [day, i] = key.split(":").map(Number);
      return { day, i, acc };
    })
    .sort((x, y) => x.day - y.day || x.i - y.i);
  const out: SlotBar[] = [];
  let run: { day: number; i0: number; i1: number; acc: Acc } | null = null;
  const flush = () => {
    if (!run) return;
    const { day, i0, i1, acc } = run;
    const top = (m: Map<string | null, number>) => [...m.entries()].sort((x, y) => y[1] - x[1]);
    out.push({
      day,
      a: i0 * step,
      b: (i1 + 1) * step,
      start: fromWallMs(i0 * step, starts[day]),
      end: fromWallMs((i1 + 1) * step, starts[day]),
      ms: acc.ms,
      categoryId: top(acc.cats)[0]?.[0] ?? null,
      titles: (top(acc.titles) as [string, number][]).slice(0, 3).map(([title, ms]) => ({ title, ms })),
    });
  };
  for (const { day, i, acc } of filled) {
    if (run && run.day === day && run.i1 === i - 1) {
      run.i1 = i;
      run.acc.ms += acc.ms;
      for (const [k, v] of acc.cats) run.acc.cats.set(k, (run.acc.cats.get(k) ?? 0) + v);
      for (const [k, v] of acc.titles) run.acc.titles.set(k, (run.acc.titles.get(k) ?? 0) + v);
    } else {
      flush();
      run = { day, i0: i, i1: i, acc: { ms: acc.ms, cats: new Map(acc.cats), titles: new Map(acc.titles) } };
    }
  }
  flush();
  return out;
}

type Hover = { bar: SlotBar; label: string; x: number; y: number };

/**
 * Uygulama çizelgesi: her uygulama bir şerit, kullanıldığı saatler çubuk.
 * Satıra tıklayınca pencere başlıkları ayrı şeritlerde açılır. Birden çok günde
 * yatay eksen günlere bölünür; her günün diliminde aynı saat aralığı gösterilir.
 * Yakınlaştırınca (`zoom`) bu aralığın bir bölümü gösterilir; yatay kaydırmayla gezilir.
 */
export default function AppTimeline({
  from,
  days = 1,
  windows,
  tags,
  onSelectDay,
  zoom = 1,
  onZoom,
  onSelectSpan,
}: {
  from: Date;
  days?: number;
  windows: WindowSpan[];
  tags: Map<string, Tag>;
  onSelectDay?: (iso: string) => void;
  zoom?: number;
  onZoom?: (zoom: number) => void;
  /** Çubuğa tıklanınca o pencerenin aralığı (atama menüsü için) ve tıklanan nokta. */
  /** Çubuğa tıklanınca: aralık ve yalnızca o şeridin uygulamaları (ya da pencereleri). */
  onSelectSpan?: (start: number, end: number, x: number, y: number, scope: EditScope & { label: string }) => void;
}) {
  const dates = useMemo(() => Array.from({ length: days + 1 }, (_, i) => addDays(from, i)), [from, days]);
  const starts = useMemo(() => dates.map(Number), [dates]);
  const { apps, rest } = useMemo(() => group(windows, tags), [windows, tags]);
  const range = useMemo(() => hourRange(windows, starts), [windows, starts]);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [hover, setHover] = useState<Hover | null>(null);

  // Konumlar duvar saatine göre: yaz saati geçişinde de saat etiketleriyle hizalı.
  // Görünen pencere: tüm aralığın 1/zoom'u, ortası `center` (tüm aralığa oranla).
  const fullStart = range.first * HOUR_MS;
  const fullSpan = (range.last - range.first) * HOUR_MS;
  const [center, setCenter] = useState(0.5);
  const windowAt = (z: number, c: number) => {
    const span = fullSpan / z;
    const start = fullStart + Math.min(fullSpan - span, Math.max(0, c * fullSpan - span / 2));
    return { start, span };
  };
  const { start: startMs, span: spanMs } = windowAt(zoom, center);
  const clamp = (x: number) => Math.min(1, Math.max(0, x));
  /** Günün `day` dilimindeki duvar saatinin (ms) şerit üzerindeki yeri (0–1). */
  const x = (day: number, wall: number) => (day + clamp((wall - startMs) / spanMs)) / days;
  const ticks = timeTicks(startMs, spanMs);

  // Hareketler art arda gelir; son değerler çizimi beklemeden zincirlensin.
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const live = useRef({ zoom, center });
  live.current = { zoom, center };
  /** İmlecin şeritteki yeri: günün dilimi içinde 0–1 ve dilimin piksel genişliği. */
  const pointer = (clientX: number) => {
    const track = root?.querySelector("[data-track]")?.getBoundingClientRect();
    if (!track) return null;
    const fx = clamp((clientX - track.left) / track.width) * days;
    return { frac: days === 1 ? fx : fx - Math.min(days - 1, Math.floor(fx)), slotPx: track.width / days };
  };
  useZoomGestures(
    root,
    (factor, clientX) => {
      if (!onZoom) return;
      const at = pointer(clientX);
      const cur = live.current;
      const z = clampZoom(cur.zoom * factor);
      const before = windowAt(cur.zoom, cur.center);
      // İmlecin altındaki saat yerinde kalsın.
      const t = before.start + (at?.frac ?? 0.5) * before.span;
      const span = fullSpan / z;
      const c = (t - (at?.frac ?? 0.5) * span - fullStart + span / 2) / fullSpan;
      live.current = { zoom: z, center: c };
      setCenter(c);
      onZoom(z);
    },
    (dx) => {
      const at = pointer(0);
      const cur = live.current;
      if (!at || cur.zoom <= 1) return;
      const span = fullSpan / cur.zoom;
      const before = windowAt(cur.zoom, cur.center);
      const start = Math.min(fullStart + fullSpan - span, Math.max(fullStart, before.start + (dx / at.slotPx) * span));
      const c = (start - fullStart + span / 2) / fullSpan;
      live.current = { ...cur, center: c };
      setCenter(c);
    },
  );
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

  // Çubuklar dilim ızgarasına oturur: birkaç saniyelik pencereler 1 px'lik şeritler olmasın.
  const slotMin = slotMinutes(days, zoom);
  /**
   * Şeridin çubukları. `byTitle`: pencere şeridi, çubuk yalnızca o pencerelerin kayıtlarına
   * dokunur; yoksa şeridin uygulamalarının (aynı dilimdeki öteki uygulamalar değişmez).
   */
  const bars = (lane: Lane, appName: string, muted = false, byTitle = false) => {
    const scope = {
      appIds: [...new Set(lane.spans.map((w) => w.appId))],
      titles: byTitle ? [...new Set(lane.spans.map((w) => w.title))] : null,
      label: byTitle ? `${appName} · ${lane.label}` : lane.label,
    };
    return slotBars(lane.spans, starts, slotMin).map((bar) => {
      if (bar.b <= startMs || bar.a >= startMs + spanMs) return null;
      const a = x(bar.day, bar.a);
      const b = x(bar.day, bar.b);
      const key = `${bar.day}:${bar.a}`;
      const label = `${appName} · ${formatTime(new Date(bar.start))}–${formatTime(new Date(bar.end))}`;
      const props = {
        className: cn(
          "absolute inset-y-1 rounded-[4px] transition-[filter] hover:brightness-95 dark:hover:brightness-125",
          muted && "opacity-70",
          onSelectSpan &&
            "cursor-pointer focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
        ),
        style: {
          left: `${a * 100}%`,
          width: `${(b - a) * 100}%`,
          background: tagColor(bar.categoryId ? tags.get(bar.categoryId) : undefined),
        },
        onMouseEnter: (e: React.MouseEvent) => setHover({ bar, label: appName, x: e.clientX, y: e.clientY }),
        onMouseMove: (e: React.MouseEvent) => setHover({ bar, label: appName, x: e.clientX, y: e.clientY }),
        onMouseLeave: () => setHover(null),
      };
      if (!onSelectSpan) return <span key={key} {...props} />;
      return (
        <button
          key={key}
          type="button"
          aria-label={`${label}: projeye ya da kategoriye ata`}
          {...props}
          onClick={(e) => {
            // Satırın açılıp kapanmasını tetiklemesin.
            e.stopPropagation();
            setHover(null);
            // Klavyeyle (Enter/Boşluk) basılınca imleç konumu yok: menü çubuğun yanında açılır.
            const r = e.currentTarget.getBoundingClientRect();
            const [px, py] = e.detail === 0 ? [r.left + r.width / 2, r.bottom] : [e.clientX, e.clientY];
            onSelectSpan(bar.start, bar.end, px, py, scope);
          }}
        />
      );
    });
  };

  const grid = (
    <>
      {days === 1
        ? ticks.map((t) => (
            <span
              key={t}
              className={cn(
                "absolute inset-y-0 border-l",
                t % HOUR_MS === 0 ? "border-border/60" : "border-dashed border-border/40",
              )}
              style={{ left: `${x(0, t) * 100}%` }}
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
    <div ref={setRoot} className="relative" onMouseLeave={() => setHover(null)}>
      <div className={cn(row, "pb-1.5 text-[10px] text-muted-foreground tabular")}>
        <span className="truncate text-[11px] font-medium">
          Uygulama
          {days > 1 && zoom > 1 && (
            <span className="font-normal text-muted-foreground">
              {" "}
              · her gün {clock(startMs)}–{clock(startMs + spanMs)}
            </span>
          )}
        </span>
        {days === 1 ? (
          <div className="relative h-4">
            {ticks.map((t) => (
              <span key={t} className="absolute -translate-x-1/2" style={{ left: `${x(0, t) * 100}%` }}>
                {zoom > 1 || t % HOUR_MS ? clock(t) : pad(t / HOUR_MS)}
              </span>
            ))}
          </div>
        ) : (
          <div className="flex" title={`Her gün ${clock(startMs)}–${clock(startMs + spanMs)}`}>
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
              {/* Satır düğme değil: içindeki çubuklar ayrı düğmeler (klavyeyle de seçilebilsin). */}
              <div className={cn(row, "py-0.5 hover:bg-accent/50")}>
                <button
                  className="flex min-w-0 items-center gap-1.5 self-stretch text-left text-xs"
                  onClick={() => toggle(app.key)}
                  aria-expanded={expanded}
                >
                  <ChevronRight
                    className={cn(
                      "size-3 shrink-0 text-muted-foreground transition-transform",
                      expanded && "rotate-90",
                    )}
                  />
                  {manual ? (
                    <PenLine className="size-3 shrink-0 text-muted-foreground" />
                  ) : (
                    <AppIcon
                      appId={app.appId}
                      name={app.label}
                      size={14}
                      fallback={<i className="size-2 shrink-0 rounded-full" style={{ background: color }} />}
                    />
                  )}
                  <span className="min-w-0 flex-1 truncate font-medium">{app.label}</span>
                  <span className="shrink-0 text-[11px] text-muted-foreground tabular">{formatDuration(app.secs)}</span>
                </button>
                {/* Çubukların arasına tıklamak da satırı açar (fareyle; klavyede soldaki düğme). */}
                <span data-track className="relative block h-8 overflow-hidden" onClick={() => toggle(app.key)}>
                  {grid}
                  {bars(app, app.label)}
                </span>
              </div>
              {expanded && (
                <ul className="pb-1">
                  {app.titles.map((t) => (
                    <li key={t.key} className={row}>
                      <span className="flex min-w-0 items-center gap-1.5 pl-[30px] text-[11px]" title={t.label}>
                        <span className="min-w-0 flex-1 truncate text-muted-foreground">{t.label}</span>
                        <span className="shrink-0 text-muted-foreground tabular">{formatDuration(t.secs)}</span>
                      </span>
                      <span className="relative block h-6 overflow-hidden">
                        {grid}
                        {bars(t, app.label, true, true)}
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
            <span className="relative block h-6 overflow-hidden">
              {grid}
              {bars(rest, rest.label, true)}
            </span>
          </li>
        )}
      </ul>

      {hover && <HoverCard hover={hover} tags={tags} />}
    </div>
  );
}

const pad = (h: number) => String(h % 24).padStart(2, "0");
/** Günün duvar saati (ms) → "09:30". */
const clock = (ms: number) => {
  const m = Math.round(ms / 60_000);
  return `${pad(Math.floor(m / 60))}:${String(m % 60).padStart(2, "0")}`;
};

/** Görünen aralıkta en çok 12 çizgi olacak en sık adımda (5/10/15/30/60 dk) zaman işaretleri. */
export function timeTicks(start: number, span: number): number[] {
  const step = [5, 10, 15, 30].map((m) => m * 60_000).find((s) => span / s <= 12) ?? HOUR_MS;
  const out = [];
  for (let t = Math.ceil(start / step) * step; t <= start + span + 1; t += step) out.push(t);
  return out;
}

function HoverCard({ hover, tags }: { hover: Hover; tags: Map<string, Tag> }) {
  const { bar } = hover;
  const tag = bar.categoryId ? tags.get(bar.categoryId) : undefined;
  // İmlecin sağında; ekranın sağına taşacaksa solunda.
  const left = hover.x + 260 > window.innerWidth ? hover.x - 252 : hover.x + 12;
  return (
    <div
      className="pointer-events-none fixed z-50 w-60 rounded-lg border bg-popover px-3 py-2 text-popover-foreground shadow-lg"
      style={{ left, top: hover.y + 14 }}
    >
      <div className="flex items-center gap-1.5 text-xs font-medium">
        <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
        <span className="min-w-0 truncate">{hover.label}</span>
      </div>
      <p className="mt-1 text-[11px] tabular">
        {formatTime(new Date(bar.start))} – {formatTime(new Date(bar.end))} · {formatDuration(bar.ms / 1000)}
        {tag && <span className="text-muted-foreground"> · {tag.name}</span>}
      </p>
      {bar.titles.some((t) => t.title) && (
        <ul className="mt-1 space-y-0.5">
          {bar.titles.map((t) => (
            <li key={t.title} className="flex gap-2 text-[11px] text-muted-foreground">
              <span className="min-w-0 flex-1 truncate">{t.title || "(başlıksız)"}</span>
              <span className="shrink-0 tabular">{formatDuration(t.ms / 1000)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
