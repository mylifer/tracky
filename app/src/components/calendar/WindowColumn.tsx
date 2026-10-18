import type { EditScope, Tag } from "../../api";
import { formatDuration } from "../../api";
import { formatTime } from "../../lib/dates";
import { UNASSIGNED, UNCATEGORIZED, tagColor } from "../../lib/tags";
import { AppIcon } from "../AppIcon";
import type { ColorLens } from "./grid";
import { itemLabel, type PlacedItem } from "./windowLanes";

/** Öğe etiketinin yüksekliği (tek satır: simge, başlık, süre). */
export const ITEM_LABEL_PX = 18;
/** Bir şeridin en dar hâli: başlık okunabilsin. */
export const LANE_MIN_PX = 170;
/** En çok bu kadar şerit: daha genişte şeritler genişler. */
export const MAX_LANES = 5;
/** Şeritlerin solundaki görüşme rayının genişliği. */
export const RAIL_PX = 10;
const ICON_PX = 14;
/** Görüşmenin rengi (takvim toplantıları gibi). */
const CALL_COLOR = "var(--primary)";

/** Sütun genişliğine sığan şerit sayısı. */
export function laneCount(width: number, rail: boolean): number {
  return Math.min(MAX_LANES, Math.max(1, Math.floor((width - (rail ? RAIL_PX : 0)) / LANE_MIN_PX)));
}

type Select = (start: number, end: number, x: number, y: number, scope?: EditScope & { label: string }) => void;

/**
 * Ayrıntı sütununun öğeleri: her pencere başladığı saatte simgesi, başlığı ve gerçekten geçen
 * süresiyle; altındaki çubuk ilk görüldüğü andan son görüldüğü ana kadar, dolu kısımları o
 * pencerede geçen anlar. Görüşmeler soldaki rayda, ön plandaki pencerelerle eş zamanlı.
 */
export function WindowItems({
  placed,
  lanes,
  rail,
  tags,
  lens,
  top,
  onSelect,
}: {
  placed: PlacedItem[];
  lanes: number;
  /** Görüşme rayı var mı (şeritler sağa kayar). */
  rail: boolean;
  tags: Map<string, Tag>;
  lens: ColorLens;
  top: (t: number) => number;
  onSelect?: Select;
}) {
  const offset = rail ? RAIL_PX : 0;
  const laneBox = (lane: number) => ({
    left: `calc(${offset}px + (100% - ${offset}px) * ${lane / lanes})`,
    width: `calc((100% - ${offset}px) / ${lanes} - 4px)`,
  });
  const pick = (e: React.MouseEvent<HTMLElement>, p: PlacedItem) => {
    if (!onSelect) return;
    // Klavyeyle basılınca imleç konumu yok: menü öğenin yanında açılır.
    const r = e.currentTarget.getBoundingClientRect();
    const [x, y] = e.detail === 0 ? [r.left + 24, r.top + ITEM_LABEL_PX] : [e.clientX, e.clientY];
    if (p.kind === "call") return onSelect(p.start, p.end, x, y);
    onSelect(p.start, p.end, x, y, {
      appIds: [p.appId],
      titles: [p.title],
      label: `${p.appName} · ${itemLabel(p)}`,
    });
  };
  return (
    <>
      {placed.map((p) => {
        const label = itemLabel(p);
        const time = `${formatTime(new Date(p.start))}–${formatTime(new Date(p.end))}`;
        const duration = formatDuration(p.ms / 1000);
        if (p.kind === "call") {
          const tip = [
            label,
            `${time} · ${duration}`,
            "Mikrofon açıkken ön planda başka pencereler de olabilir.",
            onSelect && "Tıkla: görüşme boyunca geçen süreyi ata",
          ]
            .filter(Boolean)
            .join("\n");
          return (
            <span key={p.key}>
              <button
                type="button"
                tabIndex={-1}
                aria-hidden
                disabled={!onSelect}
                title={tip}
                className="absolute left-0.5 w-1 rounded-full enabled:cursor-pointer enabled:hover:brightness-110"
                style={{ top: p.top, height: Math.max(3, p.height), background: CALL_COLOR }}
                onClick={(e) => pick(e, p)}
              />
              <button
                type="button"
                disabled={!onSelect}
                title={tip}
                className="absolute flex items-center gap-1.5 rounded-[4px] px-1 text-left enabled:cursor-pointer enabled:hover:bg-accent"
                style={{ top: p.top, height: ITEM_LABEL_PX, ...laneBox(p.lane) }}
                onClick={(e) => pick(e, p)}
              >
                <AppIcon appId={p.appId} name={p.appName} size={ICON_PX} />
                <span className="min-w-0 flex-1 truncate text-[11px] leading-tight font-medium">{label}</span>
                <span className="shrink-0 text-[10px] text-muted-foreground tabular">{duration}</span>
                <span className="sr-only">{time}</span>
              </button>
            </span>
          );
        }
        const category = p.categoryId ? tags.get(p.categoryId) : undefined;
        const project = p.projectId ? tags.get(p.projectId) : undefined;
        const color =
          lens === "category" ? tagColor(category) : project ? tagColor(project) : "var(--muted-foreground)";
        const tip = [
          label,
          p.domain && p.domain !== label ? `${p.appName} · ${p.domain}` : p.appName,
          `${time} · ${duration}${p.parts.length > 1 ? ` (${p.parts.length} kez)` : ""}`,
          `${category?.name ?? UNCATEGORIZED} · ${project?.name ?? UNASSIGNED}`,
          onSelect && "Tıkla: yalnızca bu pencerenin süresini ata",
        ]
          .filter(Boolean)
          .join("\n");
        return (
          <button
            key={p.key}
            type="button"
            disabled={!onSelect}
            title={tip}
            className="group absolute text-left enabled:cursor-pointer"
            style={{ top: p.top, height: p.height, ...laneBox(p.lane) }}
            onClick={(e) => pick(e, p)}
          >
            {/* Çubuk simgenin altından geçer: ilk ve son görüldüğü an arası soluk, geçen anlar dolu. */}
            <span
              aria-hidden
              className="absolute w-[3px] overflow-hidden rounded-full"
              style={{
                left: ICON_PX / 2 + 2.5,
                top: 0,
                bottom: 0,
                background: `color-mix(in srgb, ${color} 22%, transparent)`,
              }}
            >
              {p.parts.map(([a, b]) => (
                <span
                  key={a}
                  className="absolute inset-x-0"
                  style={{ top: top(a) - p.top, height: Math.max(1, top(b) - top(a)), background: color }}
                />
              ))}
            </span>
            <span
              className="relative flex items-center gap-1.5 rounded-[4px] px-1 group-enabled:group-hover:bg-accent"
              style={{ height: ITEM_LABEL_PX }}
            >
              <AppIcon appId={p.appId} name={p.appName} size={ICON_PX} />
              <span className="min-w-0 flex-1 truncate text-[11px] leading-tight">{label}</span>
              <span className="shrink-0 text-[10px] text-muted-foreground tabular">{duration}</span>
            </span>
            <span className="sr-only">{time}</span>
          </button>
        );
      })}
    </>
  );
}
