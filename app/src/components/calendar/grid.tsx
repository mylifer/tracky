import { useRef, useState } from "react";
import { today, wallMs } from "../../lib/dates";
import { cn } from "../../lib/utils";

/** Yakınlaştırılmamış takvimde bir saatin yüksekliği. */
export const HOUR_PX = 80;
export const HOUR_MS = 3600_000;
export const MIN = 60_000;
/** Bu yükseklikten küçük bloklarda yalnızca başlık gösterilir. */
export const FULL_LABEL_PX = 40;
/** Bundan alçak bloklarda yazı yok (yalnızca renk; ayrıntı ipucunda). */
export const LABEL_MIN_PX = 15;

/** Blokların rengi: kategoriye ya da projeye göre (takvimin "Renk" seçimi). */
export type ColorLens = "category" | "project";

/** Taralı, renksiz zemin: projeye atanmamış blok ve lejanttaki karşılığı. */
export const HATCH =
  "repeating-linear-gradient(135deg, color-mix(in srgb, var(--muted-foreground) 14%, transparent) 0 4px, transparent 4px 9px)";

/** Gösterilen saatler ve bir saatin piksel yüksekliği (yakınlaştırmayla değişir). */
type Range = { first: number; last: number; px: number };

/** Etkinliğe göre gösterilecek saat aralığı (en az 08–18). */
export function hourRange(from: Date, spans: { start: string; end: string }[], px: number, days = 1): Range {
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

export function topFn(dayStart: number, range: Range) {
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

export function HourRail({ range }: { range: Range }) {
  const marks = subMarks(range.px, true);
  return (
    // Yakınlaştırmada kaydırma bu ızgaranın üst kenarına göre sabitlenir; "Sığdır" saat sayısını buradan okur.
    <div
      data-zoom-grid
      data-hours={range.last - range.first}
      className="relative"
      style={{ height: (range.last - range.first) * range.px }}
    >
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
export const SNAP_MS = 5 * 60_000;
export const DRAG_MIN_PX = 4;

export function Column({
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
  // Sütunun dışına sürüklense de seçim görünen saatlerde kalır (önceki/sonraki güne taşmaz).
  const offsetAt = (el: HTMLElement, clientY: number) =>
    Math.min(
      Math.max(
        range.first * HOUR_MS + ((clientY - el.getBoundingClientRect().top) / range.px) * HOUR_MS,
        range.first * HOUR_MS,
      ),
      range.last * HOUR_MS,
    );
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
        // Düğme sütunun dışında bırakıldıysa basış bitmiştir: fareyle gezinmek seçim başlatmasın.
        if (p && e.buttons === 0) {
          pending.current = null;
        } else if (p && Math.abs(e.clientY - p.y0) >= DRAG_MIN_PX) {
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

export function NowLine({ day, range }: { day: Date; range: Range }) {
  if (+day !== +today()) return null;
  const top = topFn(+day, range)(Date.now());
  if (top < 0 || top > (range.last - range.first) * range.px) return null;
  return (
    <span className="pointer-events-none absolute inset-x-0 z-10 h-px bg-destructive" style={{ top }}>
      <span className="absolute -top-[3px] -left-[3px] size-[7px] rounded-full bg-destructive" />
    </span>
  );
}

export const short = (s: string) => (s.length > 40 ? `${s.slice(0, 39)}…` : s);
