import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { formatDuration } from "../api";
import { addDays, formatDate } from "../lib/dates";
import { PERIODS, type Period } from "../lib/insights";
import { cn } from "../lib/utils";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

/*
 * Rapor sayfalarının küçük grafik kitaplığı (SVG). Grafikler kapsayıcının gerçek genişliğinde
 * çizilir (yazılar ölçeklenmesin); değerler üstüne gelince ipucunda görünür. Süreler saniye.
 */

const hoursFmt = new Intl.NumberFormat("tr-TR", { maximumFractionDigits: 1 });
/** Eksen etiketi: saniyeyi saate çevirir ("12,5"). */
export const hours = (secs: number) => hoursFmt.format(secs / 3600);

/** Kapsayıcının genişliği (yeniden boyutlanınca güncellenir). */
function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

/** Fareyi izleyen ipucu; kapsayıcıya göre konumlanır, sağ kenarda sola döner. */
function useTip() {
  const ref = useRef<HTMLDivElement>(null);
  const [tip, setTip] = useState<{ x: number; y: number; body: ReactNode } | null>(null);
  const show = (e: React.PointerEvent, body: ReactNode) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setTip({ x: e.clientX - r.left, y: e.clientY - r.top, body });
  };
  const flip = tip && ref.current ? tip.x > ref.current.clientWidth * 0.6 : false;
  const node = tip && (
    <div
      className="pointer-events-none absolute z-30 w-max max-w-64 rounded-lg border bg-popover px-2.5 py-1.5 text-[11px] leading-snug text-popover-foreground shadow-md"
      style={{
        left: tip.x + (flip ? -12 : 12),
        top: tip.y + 12,
        transform: flip ? "translateX(-100%)" : undefined,
      }}
    >
      {tip.body}
    </div>
  );
  return { ref, show, hide: () => setTip(null), node };
}

function niceMax(v: number): number {
  if (v <= 0) return 3600;
  const p = 10 ** Math.floor(Math.log10(v));
  const n = v / p;
  return (n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10) * p;
}

/** Üst sınırı yuvarlak adımlara bölen çizgi sayısı (10 → 5 × 2, 2,5 → 5 × 0,5, 2 → 4 × 0,5). */
function tickCount(max: number): number {
  const lead = max / 10 ** Math.floor(Math.log10(max));
  return lead === 2 || lead === 1 ? 4 : 5;
}

/** Saat ekseninin üst sınırı: tam saatlere yuvarlanır. */
function hourMax(secs: number): number {
  return niceMax(secs / 3600) * 3600;
}

export type Series = { key: string; name: string; color: string; values: number[] };

const AXIS_W = 30;

function YAxis({ max, height, top, width }: { max: number; height: number; top: number; width: number }) {
  const ticks = tickCount(max / 3600);
  return (
    <g>
      {Array.from({ length: ticks + 1 }, (_, i) => {
        const v = (max / ticks) * i;
        const y = top + height - (v / max) * height;
        return (
          <g key={i}>
            <line x1={AXIS_W} x2={width} y1={y} y2={y} stroke="var(--border)" />
            <text x={AXIS_W - 6} y={y + 3} textAnchor="end" className="fill-muted-foreground text-[10px] tabular">
              {hours(v)}
            </text>
          </g>
        );
      })}
    </g>
  );
}

function XLabels({ labels, x, y, width }: { labels: string[]; x: (i: number) => number; y: number; width: number }) {
  const n = labels.length;
  // Etiketler arası en az ~56 piksel; son etiket hep görünür, ona çok yakın olan atlanır.
  const every = Math.max(1, Math.ceil(n / Math.max(1, (width - AXIS_W) / 56)));
  return (
    <g>
      {labels.map((l, i) =>
        (i % every === 0 && n - 1 - i >= every / 2) || i === n - 1 ? (
          // Kenara yakın etiket taşmasın: sağ uçta sağa, sol uçta sola yaslanır.
          <text
            key={i}
            x={x(i) > width - 24 ? width : x(i)}
            y={y}
            textAnchor={x(i) > width - 24 ? "end" : "middle"}
            className="fill-muted-foreground text-[10px]"
          >
            {l}
          </text>
        ) : null,
      )}
    </g>
  );
}

/**
 * Yığılmış sütun grafik (saat ekseni). `partial` son sütunu soluk çizer (süren dönem).
 * `tipLabels` ipucundaki uzun dönem adları.
 */
export function StackedBars({
  labels,
  tipLabels,
  series,
  height = 190,
  partial,
}: {
  labels: string[];
  tipLabels?: string[];
  series: Series[];
  height?: number;
  partial?: boolean;
}) {
  const [wref, width] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const T = 8,
    B = 20,
    H = height - T - B;
  const n = labels.length;
  const totals = labels.map((_, i) => series.reduce((s, x) => s + (x.values[i] ?? 0), 0));
  const max = hourMax(Math.max(...totals, 0));
  const bw = (width - AXIS_W) / Math.max(1, n);
  const gap = Math.max(2, Math.min(10, bw * 0.3));
  const x = (i: number) => AXIS_W + i * bw + bw / 2;
  return (
    <div ref={tip.ref} className="relative">
      <div ref={wref} onPointerLeave={tip.hide}>
        {width > 0 && (
          <svg width={width} height={height} role="img" className="block">
            <YAxis max={max} height={H} top={T} width={width} />
            {labels.map((_, i) => {
              let y = T + H;
              const w = Math.max(1, bw - gap);
              const left = x(i) - w / 2;
              const visible = series.filter((s) => (s.values[i] ?? 0) > 0);
              return (
                <g key={i} opacity={partial && i === n - 1 ? 0.45 : 1}>
                  {visible.map((s, k) => {
                    const h = ((s.values[i] ?? 0) / max) * H;
                    y -= h;
                    const top = k === visible.length - 1;
                    // Üst uç yuvarlak; dilimler arasında yüzey renginde ince boşluk.
                    return top ? (
                      <path key={s.key} d={roundTop(left, y, w, h, Math.min(3, w / 2, h))} fill={s.color} />
                    ) : (
                      <rect key={s.key} x={left} y={y + 1} width={w} height={Math.max(0, h - 1)} fill={s.color} />
                    );
                  })}
                </g>
              );
            })}
            <XLabels labels={labels} x={x} y={height - 5} width={width} />
            {labels.map((l, i) => (
              <rect
                key={i}
                x={AXIS_W + i * bw}
                y={T}
                width={bw}
                height={H}
                fill="transparent"
                onPointerMove={(e) =>
                  tip.show(
                    e,
                    <>
                      <div className="font-medium">
                        {tipLabels?.[i] ?? l} · {formatDuration(totals[i])}
                      </div>
                      {series
                        .filter((s) => (s.values[i] ?? 0) > 0)
                        .map((s) => (
                          <TipRow key={s.key} color={s.color} name={s.name} value={formatDuration(s.values[i])} />
                        ))}
                    </>,
                  )
                }
              />
            ))}
          </svg>
        )}
      </div>
      {tip.node}
    </div>
  );
}

function roundTop(x: number, y: number, w: number, h: number, r: number) {
  if (h <= 0) return "";
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

function TipRow({ color, name, value }: { color: string; name: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5">
      <i className="size-1.5 shrink-0 rounded-full" style={{ background: color }} />
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{name}</span>
      <span className="tabular">{value}</span>
    </div>
  );
}

/** Çok serili çizgi grafik (saat ekseni); tek seride alan dolgusu. */
export function LineChart({
  labels,
  tipLabels,
  series,
  height = 200,
}: {
  labels: string[];
  tipLabels?: string[];
  series: Series[];
  height?: number;
}) {
  const [wref, width] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const [hover, setHover] = useState<number | null>(null);
  const T = 10,
    B = 20,
    R = 8,
    H = height - T - B;
  const n = labels.length;
  const max = hourMax(Math.max(0, ...series.flatMap((s) => s.values)));
  const x = (i: number) => AXIS_W + 8 + (i / Math.max(1, n - 1)) * (width - AXIS_W - 8 - R);
  const y = (v: number) => T + H - (v / max) * H;
  const step = (width - AXIS_W) / Math.max(1, n);
  return (
    <div ref={tip.ref} className="relative">
      <div
        ref={wref}
        onPointerLeave={() => {
          tip.hide();
          setHover(null);
        }}
      >
        {width > 0 && (
          <svg width={width} height={height} role="img" className="block">
            <YAxis max={max} height={H} top={T} width={width} />
            {hover !== null && <line x1={x(hover)} x2={x(hover)} y1={T} y2={T + H} stroke="var(--border)" />}
            {series.map((s) => {
              const d = s.values.map((v, i) => `${i ? "L" : "M"}${x(i)},${y(v)}`).join("");
              return (
                <g key={s.key}>
                  {series.length === 1 && (
                    <path d={`${d}L${x(n - 1)},${T + H}L${x(0)},${T + H}Z`} fill={s.color} opacity={0.12} />
                  )}
                  <path
                    d={d}
                    fill="none"
                    stroke={s.color}
                    strokeWidth={2}
                    strokeLinejoin="round"
                    strokeLinecap="round"
                  />
                  <circle
                    cx={x(hover ?? n - 1)}
                    cy={y(s.values[hover ?? n - 1] ?? 0)}
                    r={3.5}
                    fill={s.color}
                    stroke="var(--card)"
                    strokeWidth={2}
                  />
                </g>
              );
            })}
            <XLabels labels={labels} x={x} y={height - 5} width={width} />
            {labels.map((l, i) => (
              <rect
                key={i}
                x={x(i) - step / 2}
                y={T}
                width={step}
                height={H}
                fill="transparent"
                onPointerMove={(e) => {
                  setHover(i);
                  tip.show(
                    e,
                    <>
                      <div className="font-medium">{tipLabels?.[i] ?? l}</div>
                      {series.map((s) => (
                        <TipRow key={s.key} color={s.color} name={s.name} value={formatDuration(s.values[i] ?? 0)} />
                      ))}
                    </>,
                  );
                }}
              />
            ))}
          </svg>
        )}
      </div>
      {tip.node}
    </div>
  );
}

/** Küçük eğilim çizgisi: alan dolgusu ve vurgulu son nokta. */
export function Sparkline({
  values,
  color,
  height = 32,
  className,
}: {
  values: number[];
  color: string;
  height?: number;
  className?: string;
}) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const n = values.length;
  const max = Math.max(...values, 1);
  const pts = values.map((v, i) => [2 + (i / Math.max(1, n - 1)) * (width - 6), height - 3 - (v / max) * (height - 7)]);
  const d = pts.map((p, i) => `${i ? "L" : "M"}${p[0]},${p[1]}`).join("");
  const last = pts[n - 1];
  return (
    <div ref={ref} className={cn("w-full", className)} style={{ height }} aria-hidden>
      {width > 0 && n > 1 && (
        <svg width={width} height={height} className="block overflow-visible">
          <path d={`${d}L${last[0]},${height}L${pts[0][0]},${height}Z`} fill={color} opacity={0.13} />
          <path d={d} fill="none" stroke={color} strokeWidth={1.6} strokeLinejoin="round" />
          <circle cx={last[0]} cy={last[1]} r={2.5} fill={color} />
        </svg>
      )}
    </div>
  );
}

export type Share = { key: string; name: string; color: string; value: number };

/** %100 yatay pay çubuğu; dilimler arası ince boşluk. */
export function ShareBar({ items, format = formatDuration }: { items: Share[]; format?: (v: number) => string }) {
  const tip = useTip();
  const total = items.reduce((s, i) => s + i.value, 0) || 1;
  return (
    <div ref={tip.ref} className="relative">
      <div className="flex h-2.5 gap-0.5 overflow-hidden rounded-full bg-muted" onPointerLeave={tip.hide}>
        {items
          .filter((i) => i.value > 0)
          .map((i) => (
            <i
              key={i.key}
              className="block h-full"
              style={{ width: `${(i.value / total) * 100}%`, background: i.color }}
              onPointerMove={(e) =>
                tip.show(
                  e,
                  <TipRow
                    color={i.color}
                    name={i.name}
                    value={`${format(i.value)} · %${Math.round((i.value / total) * 100)}`}
                  />,
                )
              }
            />
          ))}
      </div>
      {tip.node}
    </div>
  );
}

/** Sıralı liste: ad, değer ve en büyüğe göre oranlı ince çubuk. */
export function RankList({
  items,
  format = formatDuration,
  empty = "Kayıt yok",
}: {
  items: (Share & { title?: string; sub?: string })[];
  format?: (v: number) => string;
  empty?: string;
}) {
  if (items.length === 0) return <p className="text-xs text-muted-foreground">{empty}</p>;
  const max = Math.max(...items.map((i) => i.value), 1);
  return (
    <ul className="space-y-2">
      {items.map((i) => (
        <li key={i.key} className="space-y-1" title={i.title}>
          <div className="flex items-baseline gap-2 text-xs">
            <i className="size-2 shrink-0 self-center rounded-full" style={{ background: i.color }} />
            <span className="min-w-0 flex-1 truncate">
              {i.name}
              {i.sub && <span className="text-muted-foreground"> · {i.sub}</span>}
            </span>
            <span className="shrink-0 text-muted-foreground tabular">{format(i.value)}</span>
          </div>
          <div className="h-1 overflow-hidden rounded-full bg-muted">
            <i
              className="block h-full rounded-full"
              style={{ width: `${(i.value / max) * 100}%`, background: i.color }}
            />
          </div>
        </li>
      ))}
    </ul>
  );
}

const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric", month: "long" });
const WEEKDAYS = ["Pzt", "", "Çar", "", "Cum", "", ""];

/**
 * Gün gün ısı haritası: sütunlar haftalar, satırlar Pazartesi–Pazar. `start` bir Pazartesi;
 * bugünden sonraki günler boş çizilmez.
 */
export function Heatmap({ start, days, color }: { start: Date; days: number[]; color: string }) {
  const tip = useTip();
  const weeks = Math.ceil(days.length / 7);
  const max = Math.max(...days, 1);
  const now = Date.now();
  return (
    <div ref={tip.ref} className="relative w-fit max-w-full space-y-2">
      <div className="flex gap-1.5" onPointerLeave={tip.hide}>
        <div className="grid shrink-0 grid-rows-7 gap-[3px] text-[9px] leading-none text-muted-foreground">
          {WEEKDAYS.map((d, i) => (
            <span key={i} className="flex items-center">
              {d}
            </span>
          ))}
        </div>
        <div
          className="grid min-w-0 flex-1 grid-flow-col grid-rows-7 gap-[3px]"
          // Hücreler en çok 16 piksel: geniş pencerede kocaman kareler olmasın.
          style={{ gridTemplateColumns: `repeat(${weeks}, minmax(0, 16px))` }}
        >
          {days.map((v, i) => {
            const date = addDays(start, i);
            if (date.getTime() > now) return <span key={i} className="aspect-square" />;
            const pct = v > 0 ? Math.round(22 + 78 * (v / max)) : 0;
            return (
              <span
                key={i}
                className="aspect-square rounded-[3px] bg-muted"
                style={v > 0 ? { background: `color-mix(in srgb, ${color} ${pct}%, var(--muted))` } : undefined}
                onPointerMove={(e) =>
                  tip.show(
                    e,
                    <>
                      <div className="font-medium">{longDate.format(date)}</div>
                      <div className="text-muted-foreground">{v > 0 ? formatDuration(v) : "Kayıt yok"}</div>
                    </>,
                  )
                }
              />
            );
          })}
        </div>
      </div>
      <div className="flex items-center justify-between pl-6 text-[10px] text-muted-foreground">
        <span>{formatDate(start)}</span>
        <span className="flex items-center gap-1">
          Az
          {[25, 55, 100].map((p) => (
            <i
              key={p}
              className="size-2 rounded-[2px]"
              style={{ background: `color-mix(in srgb, ${color} ${p}%, var(--muted))` }}
            />
          ))}
          Çok
        </span>
        <span>bugün</span>
      </div>
    </div>
  );
}

/** Günün saatlerine dağılım (24 öğe); yalnızca çalışılan saat aralığı çizilir. */
export function HourBars({ hours: secs, color, height = 110 }: { hours: number[]; color: string; height?: number }) {
  const [wref, width] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const active = secs.map((v, i) => (v > 0 ? i : -1)).filter((i) => i >= 0);
  const from = Math.min(8, active[0] ?? 8);
  const to = Math.max(18, active[active.length - 1] ?? 18);
  const shown = secs.slice(from, to + 1);
  const total = secs.reduce((a, b) => a + b, 0) || 1;
  const max = Math.max(...shown, 1);
  const B = 16,
    H = height - B - 4;
  const bw = width / Math.max(1, shown.length);
  return (
    <div ref={tip.ref} className="relative">
      <div ref={wref} onPointerLeave={tip.hide}>
        {width > 0 && (
          <svg width={width} height={height} role="img" className="block">
            {shown.map((v, i) => {
              const h = (v / max) * H;
              const w = Math.max(2, bw - Math.min(6, bw * 0.3));
              const x = i * bw + (bw - w) / 2;
              const hour = from + i;
              return (
                <g key={i}>
                  {h > 0 && <path d={roundTop(x, 4 + H - h, w, h, Math.min(3, w / 2, h))} fill={color} />}
                  {(i % 2 === 0 || bw > 28) && (
                    <text
                      x={x + w / 2}
                      y={height - 3}
                      textAnchor="middle"
                      className="fill-muted-foreground text-[10px] tabular"
                    >
                      {hour}
                    </text>
                  )}
                  <rect
                    x={i * bw}
                    y={0}
                    width={bw}
                    height={height}
                    fill="transparent"
                    onPointerMove={(e) =>
                      tip.show(
                        e,
                        <>
                          <div className="font-medium">
                            {String(hour).padStart(2, "0")}:00–{String(hour + 1).padStart(2, "0")}:00
                          </div>
                          <div className="text-muted-foreground">
                            {formatDuration(v)} · %{Math.round((v / total) * 100)}
                          </div>
                        </>,
                      )
                    }
                  />
                </g>
              );
            })}
          </svg>
        )}
      </div>
      {tip.node}
    </div>
  );
}

/**
 * Bütçe grafiği: kümülatif harcanan adam-gün (dönem başındaki harcama dahil), bütçe çizgisi ve
 * son 4 haftanın hızıyla kesikli tahmin. `weekly` son öğe süren hafta.
 */
export function BurnChart({
  periods,
  weekly,
  usedSeconds,
  budgetSeconds,
  dayHours,
  color,
  height = 190,
}: {
  periods: string[];
  weekly: number[];
  usedSeconds: number;
  budgetSeconds: number;
  dayHours: number;
  color: string;
  height?: number;
}) {
  const [wref, width] = useWidth<HTMLDivElement>();
  const tip = useTip();
  const day = (dayHours || 8) * 3600;
  // Nokta i: i. haftanın sonundaki kümülatif harcama (son nokta: şimdi).
  const cum: number[] = [];
  let acc = usedSeconds;
  for (let i = weekly.length - 1; i >= 0; i--) {
    cum[i] = acc;
    acc -= weekly[i];
  }
  const done = weekly.slice(0, -1).slice(-4);
  const rate = done.reduce((a, b) => a + b, 0) / (done.length || 1);
  const AHEAD = 12;
  const forecast = Array.from({ length: AHEAD + 1 }, (_, i) => usedSeconds + rate * i);
  const n = weekly.length;
  const total = n + AHEAD;
  const T = 12,
    B = 20,
    R = 62,
    H = height - T - B;
  const max = niceMax((Math.max(budgetSeconds, ...cum, forecast[AHEAD]) * 1.05) / day) * day;
  const x = (i: number) => AXIS_W + 8 + (i / Math.max(1, total - 1)) * (width - AXIS_W - 8 - R);
  const y = (v: number) => T + H - (v / max) * H;
  const weekStart = (i: number) => addDays(new Date(periods[0]), i * 7);
  const labels = Array.from({ length: total }, (_, i) => formatDate(weekStart(i)));
  const d = cum.map((v, i) => `${i ? "L" : "M"}${x(i)},${y(v)}`).join("");
  const hit = rate > 0 ? forecast.findIndex((v) => v >= budgetSeconds) : -1;
  const fmt = (v: number) => hoursFmt.format(v / day);
  const step = (width - AXIS_W) / total;
  return (
    <div ref={tip.ref} className="relative">
      <div ref={wref} onPointerLeave={tip.hide}>
        {width > 0 && (
          <svg width={width} height={height} role="img" className="block">
            {Array.from({ length: tickCount(max / day) + 1 }, (_, i) => {
              const v = (max / tickCount(max / day)) * i;
              return (
                <g key={i}>
                  <line x1={AXIS_W} x2={width - R + 8} y1={y(v)} y2={y(v)} stroke="var(--border)" />
                  <text
                    x={AXIS_W - 6}
                    y={y(v) + 3}
                    textAnchor="end"
                    className="fill-muted-foreground text-[10px] tabular"
                  >
                    {fmt(v)}
                  </text>
                </g>
              );
            })}
            <path d={`${d}L${x(n - 1)},${T + H}L${x(0)},${T + H}Z`} fill={color} opacity={0.12} />
            <path d={d} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" />
            {rate > 0 && (
              <path
                d={forecast.map((v, i) => `${i ? "L" : "M"}${x(n - 1 + i)},${y(v)}`).join("")}
                fill="none"
                stroke={color}
                strokeWidth={2}
                strokeDasharray="4 4"
              />
            )}
            <line
              x1={AXIS_W}
              x2={width - R + 8}
              y1={y(budgetSeconds)}
              y2={y(budgetSeconds)}
              stroke="var(--destructive)"
              strokeWidth={1.5}
              strokeDasharray="2 3"
            />
            <text x={width - R + 12} y={y(budgetSeconds) + 3} className="fill-destructive text-[10px]">
              Bütçe {fmt(budgetSeconds)}
            </text>
            {hit >= 0 && (
              <circle
                cx={x(n - 1 + hit)}
                cy={y(budgetSeconds)}
                r={4}
                fill="var(--destructive)"
                stroke="var(--card)"
                strokeWidth={2}
              />
            )}
            <circle cx={x(n - 1)} cy={y(usedSeconds)} r={3.5} fill={color} stroke="var(--card)" strokeWidth={2} />
            <XLabels labels={labels} x={x} y={height - 5} width={width - R} />
            {labels.map((l, i) => (
              <rect
                key={i}
                x={x(i) - step / 2}
                y={T}
                width={step}
                height={H}
                fill="transparent"
                onPointerMove={(e) =>
                  tip.show(
                    e,
                    i < n ? (
                      <>
                        <div className="font-medium">{i === n - 1 ? "Şimdi" : `${l} haftası sonu`}</div>
                        <div className="text-muted-foreground">
                          {fmt(cum[i])} / {fmt(budgetSeconds)} adam-gün
                        </div>
                      </>
                    ) : (
                      <>
                        <div className="font-medium">{l} haftası (tahmin)</div>
                        <div className="text-muted-foreground">
                          {rate > 0 ? `${fmt(forecast[i - n + 1])} adam-gün` : "Son 4 haftada süre yok"}
                        </div>
                      </>
                    ),
                  )
                }
              />
            ))}
          </svg>
        )}
      </div>
      {tip.node}
    </div>
  );
}

/** Lejant: renk noktası ve ad (iki ve daha çok seride). */
export function Legend({ items }: { items: { key: string; name: string; color: string }[] }) {
  return (
    <div className="mt-2 flex flex-wrap gap-x-3.5 gap-y-1 text-[11px] text-muted-foreground">
      {items.map((i) => (
        <span key={i.key} className="inline-flex items-center gap-1.5">
          <i className="size-2 rounded-full" style={{ background: i.color }} />
          {i.name}
        </span>
      ))}
    </div>
  );
}

/** Değişim oranı: ▲ yeşil, ▼ kırmızı; önceki dönem boşsa "yeni". */
export function Delta({ value, className }: { value: number | null; className?: string }) {
  if (value === null) return <span className={cn("text-muted-foreground", className)}>yeni</span>;
  if (Math.abs(value) < 0.005) return <span className={cn("text-muted-foreground", className)}>±%0</span>;
  return (
    <span className={cn("tabular", value > 0 ? "text-success" : "text-destructive", className)}>
      {value > 0 ? "▲" : "▼"} %{Math.round(Math.abs(value) * 100)}
    </span>
  );
}

/** Kıyas notu: "▲ %12 geçen ayın aynı gününe göre"; önceki dönem boşsa bunu söyler. */
export function Versus({ cur, prev, text }: { cur: number; prev: number; text: string }) {
  const c = prev > 0 ? (cur - prev) / prev : null;
  if (c === null) return <>{cur > 0 ? "Önceki dönemde kayıt yok" : "Bu dönemde kayıt yok"}</>;
  return (
    <>
      <Delta value={c} /> {text}
    </>
  );
}

/** Özet rakam kutusu. */
export function Stat({
  label,
  value,
  unit,
  hint,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  hint?: ReactNode;
}) {
  return (
    <div className="min-w-0 rounded-xl border bg-card px-4 py-3 shadow-xs">
      <div className="truncate text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-0.5 text-[22px] leading-tight font-semibold tracking-tight tabular">
        {value}
        {unit && <span className="ml-0.5 text-xs font-medium text-muted-foreground">{unit}</span>}
      </div>
      {hint && <div className="mt-0.5 truncate text-[11px] text-muted-foreground">{hint}</div>}
    </div>
  );
}

/** Başlıklı grafik kartı. */
export function ChartCard({
  title,
  note,
  children,
  className,
}: {
  title: string;
  note?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("min-w-0 rounded-xl border bg-card px-4 py-3.5 shadow-xs", className)}>
      <div className="mb-2.5 flex items-baseline gap-2">
        <h3 className="text-[13px] font-semibold">{title}</h3>
        {note && <span className="ml-auto truncate text-[11px] text-muted-foreground">{note}</span>}
      </div>
      {children}
    </section>
  );
}

/** Dönem seçici (Müşteriler ve Projeler). */
export function PeriodTabs({ value, onChange }: { value: Period; onChange: (p: Period) => void }) {
  return (
    <Tabs value={value} onValueChange={(v) => onChange(v as Period)}>
      <TabsList aria-label="Dönem">
        {PERIODS.map((p) => (
          <TabsTrigger key={p.id} value={p.id} className="px-3">
            {p.label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}

/** Hafta başlarından eksen ve ipucu etiketleri; son hafta "bu hafta". */
export function weekLabels(periods: string[]) {
  const last = periods.length - 1;
  return {
    labels: periods.map((p) => formatDate(new Date(p))),
    tipLabels: periods.map((p, i) => {
      const start = new Date(p);
      return i === last ? "Bu hafta (sürüyor)" : `${formatDate(start)} – ${formatDate(addDays(start, 6))}`;
    }),
  };
}
