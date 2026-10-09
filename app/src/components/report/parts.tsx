/** Rapor ekranının küçük parçaları: bilgisayar filtresi, boş durum, yakınlaştırma, takvim
 * görünümü ve renk merceği tercihleri, proje açıklaması. */
import { useEffect, useState } from "react";
import { Hourglass, ZoomIn, ZoomOut } from "lucide-react";
import { type Bucket, type DeviceTotal, formatDuration, type Tag } from "../../api";
import { DeviceIcon } from "../DeviceIcon";
import { tagColor, UNASSIGNED } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { stepZoom } from "../../lib/zoom";
import { Dot } from "../Breakdown";
import { type ColorLens, HATCH } from "../Calendar";
import { Button } from "../ui/button";

/**
 * Bilgisayar filtresi: seçilince rapor yalnızca o bilgisayarın kaydettiği süreyi gösterir.
 * Yalnızca aralıkta birden çok bilgisayar varsa görünür.
 */
export function DeviceFilter({
  devices,
  value,
  onChange,
}: {
  devices: DeviceTotal[];
  value: string | null;
  onChange: (id: string | null) => void;
}) {
  const selected = devices.find((d) => d.id === value);
  const chip = (on: boolean) =>
    cn(
      "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-ring",
      on ? "border-foreground bg-foreground text-background" : "bg-card hover:bg-accent",
    );
  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Bilgisayar">
        <button className={chip(value === null)} aria-pressed={value === null} onClick={() => onChange(null)}>
          Tümü
        </button>
        {devices.map((d) => (
          <button
            key={d.id}
            className={chip(value === d.id)}
            aria-pressed={value === d.id}
            title={d.current ? "Bu bilgisayar" : undefined}
            onClick={() => onChange(value === d.id ? null : d.id)}
          >
            <DeviceIcon id={d.id} os={d.os} model={d.model} className="size-5" />
            {d.name}
            <span className={cn("tabular", value === d.id ? "opacity-70" : "text-muted-foreground")}>
              {formatDuration(d.seconds)}
            </span>
          </button>
        ))}
      </div>
      {selected && (
        <p className="text-[11px] text-muted-foreground">
          Yalnızca {selected.name} kayıtları gösteriliyor. Zaman çizelgesi ve takvimde yapılan düzenlemeler tüm
          bilgisayarları kapsar.
        </p>
      )}
    </div>
  );
}

/** Kayıt yokken takvimin üstünde: ne olacağını ve elle eklemenin yolunu söyler. */
export function Empty({ future }: { future: boolean }) {
  return (
    <div className="mx-1 mb-3 flex items-center gap-3 rounded-lg bg-muted/60 px-3 py-2.5">
      <Hourglass className="size-4 shrink-0 text-muted-foreground" />
      <div className="text-xs">
        <p className="font-medium">Bu aralık için kayıt yok</p>
        <p className="text-muted-foreground">
          {future
            ? "Kum arka planda çalışırken takvim kendiliğinden dolacak."
            : "Kum çalışırken takvim kendiliğinden dolar. Bilgisayar dışında geçen süre için boş alana tıkla."}
        </p>
      </div>
    </div>
  );
}

/** −  %100  + : yakınlaştırma düğmeleri; ortadaki değer sığdırır (takvimin bütün saatleri ekranda). */
export function ZoomControl({
  zoom,
  min,
  fit,
  onZoom,
  onFit,
  off,
}: {
  zoom: number;
  min: number;
  /** Sığdır açık: ölçek pencereye göre kendiliğinden ayarlanıyor. */
  fit: boolean;
  onZoom: (next: (z: number) => number) => void;
  onFit: () => void;
  /** Yakınlaştırma bu görünümde yoksa nedeni; düğmeler yerinde soluk kalır. */
  off: string | null;
}) {
  return (
    <div
      className="flex h-7 items-center rounded-md border"
      role="group"
      aria-label="Yakınlaştırma"
      title={off ?? undefined}
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-6.5"
        onClick={() => onZoom((z) => stepZoom(z, -1, min))}
        disabled={!!off || zoom <= min + 0.001}
        aria-label="Uzaklaştır"
        title="Uzaklaştır (−)"
      >
        <ZoomOut />
      </Button>
      <button
        className={cn(
          "w-10 text-center text-[11px] tabular hover:text-foreground disabled:pointer-events-none disabled:opacity-50",
          fit && !off ? "font-medium text-primary" : "text-muted-foreground",
        )}
        onClick={onFit}
        disabled={!!off}
        aria-pressed={fit}
        title={
          off
            ? undefined
            : fit
              ? "Sığdırıldı: bütün saatler ekranda · yakınlaştırınca kapanır"
              : "Sığdır (0): bütün saatler ekrana sığsın · ⌘ + kaydırma ya da iki parmakla da yakınlaşır"
        }
      >
        %{Math.round(zoom * 100)}
      </button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-6.5"
        onClick={() => onZoom((z) => stepZoom(z, 1, min))}
        disabled={!!off || zoom >= 8}
        aria-label="Yakınlaştır"
        title="Yakınlaştır (+)"
      >
        <ZoomIn />
      </Button>
    </div>
  );
}

/** Sığdırırken takvimin altında bırakılan pay: kartın ve sayfanın alt boşluğu. */
export const FIT_BOTTOM_PX = 40;
const FIT_KEY = "kum.calendarFit";

/** Takvim "Sığdır"da mı; tercih bu cihazda hatırlanır. */
export function useCalendarFit(): [boolean, (v: boolean) => void] {
  const [fit, setFit] = useState(() => {
    try {
      return localStorage.getItem(FIT_KEY) === "1";
    } catch {
      return false;
    }
  });
  return [
    fit,
    (v) => {
      setFit(v);
      try {
        localStorage.setItem(FIT_KEY, v ? "1" : "0");
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}

export type CalendarView = "calendar" | "apps" | "sheet";
// Anahtar eski adıyla kalır: kayıtlı tercih korunsun.
const CALENDAR_VIEW_KEY = "kum.dayView";

/**
 * Gün ve hafta görünümünde takvim, uygulama çizelgesi ya da (yalnızca gün) takvimle zaman
 * çizelgesi; tercih bu cihazda hatırlanır.
 */
export function useCalendarView(): [CalendarView, (v: CalendarView) => void] {
  const [view, setView] = useState<CalendarView>(() => {
    try {
      const v = localStorage.getItem(CALENDAR_VIEW_KEY);
      return v === "apps" || v === "sheet" ? v : "calendar";
    } catch {
      return "calendar";
    }
  });
  return [
    view,
    (v) => {
      setView(v);
      try {
        localStorage.setItem(CALENDAR_VIEW_KEY, v);
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}

/** Proje merceğinde lejant: projeler ve dönemdeki toplamları; atanmamış süre taralı ve sonda. */
export function ProjectLegend({ buckets, tags }: { buckets: Bucket[]; tags: Map<string, Tag> }) {
  const rows = [...buckets.filter((b) => b.id !== null), ...buckets.filter((b) => b.id === null)];
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1">
      {rows.map((b) => {
        const tag = b.id ? tags.get(b.id) : undefined;
        return (
          <li
            key={b.id ?? "none"}
            className="flex items-center gap-1.5 text-[11px] whitespace-nowrap text-muted-foreground"
          >
            {b.id ? (
              <Dot color={tagColor(tag)} />
            ) : (
              <i
                className="inline-block size-2 shrink-0 rounded-full border border-muted-foreground/40"
                style={{ background: HATCH }}
              />
            )}
            {b.id ? (tag?.name ?? "Silinen proje") : UNASSIGNED}
            <span className="tabular">{formatDuration(b.seconds)}</span>
          </li>
        );
      })}
    </ul>
  );
}

const LENS_KEY = "kum.calendarLens";

/** Takvim blokları kategori, proje ya da çizelge satırı renginde; tercih bu cihazda hatırlanır. */
export function useColorLens(): [ColorLens, (v: ColorLens) => void] {
  const [lens, setLens] = useState<ColorLens>(() => {
    try {
      const v = localStorage.getItem(LENS_KEY);
      return v === "project" || v === "sheet" ? v : "category";
    } catch {
      return "category";
    }
  });
  return [
    lens,
    (v) => {
      setLens(v);
      try {
        localStorage.setItem(LENS_KEY, v);
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}

/** Öğenin yüksekliği (değiştikçe güncellenir); öğe yokken 0. */
export function useHeight(el: HTMLElement | null) {
  const [height, setHeight] = useState(0);
  useEffect(() => {
    if (!el) return setHeight(0);
    const ro = new ResizeObserver(() => setHeight(el.offsetHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return height;
}
