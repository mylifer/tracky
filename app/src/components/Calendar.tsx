import { useMemo, useRef, useState } from "react";
import { Briefcase, CalendarDays, ChevronDown, Coffee, FolderInput, Shapes, Video } from "lucide-react";
import type { CalendarMeeting, IdleSpan, Segment, Tag, WindowSpan, WorkBlock } from "../api";
import { api, formatDuration, NO_PROJECT } from "../api";
import { blockWindows, type BlockApp, type BlockWindow } from "../lib/blockWindows";
import { detailRows, onlyOther, type DetailRow } from "../lib/minorWindows";
import { friendlyError, toast, undoable } from "../lib/feedback";
import { UNASSIGNED_MIN } from "../lib/timesheet";
import { addDays, formatTime, fromWallMs, isoDate, today, wallMs } from "../lib/dates";
import { UNASSIGNED, UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";
import { AppIcon, AppIconStack, SiteIcon } from "./AppIcon";
import { BlockActions, useEdit } from "./SessionEdit";
import { ProjectSelect } from "./ProjectSelect";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

/** Yakınlaştırılmamış takvimde bir saatin yüksekliği. */
export const HOUR_PX = 80;
const HOUR_MS = 3600_000;
/** Bu yükseklikten küçük bloklarda yalnızca başlık gösterilir. */
const FULL_LABEL_PX = 40;
/** Bundan alçak bloklarda yazı yok (yalnızca renk; ayrıntı ipucunda). */
const LABEL_MIN_PX = 15;

/** Blokların rengi: kategoriye ya da projeye göre (takvimin "Renk" seçimi). */
export type ColorLens = "category" | "project";

/** Taralı, renksiz zemin: projeye atanmamış blok ve lejanttaki karşılığı. */
export const HATCH =
  "repeating-linear-gradient(135deg, color-mix(in srgb, var(--muted-foreground) 14%, transparent) 0 4px, transparent 4px 9px)";

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
function BlockDetails({
  block,
  tags,
  windows,
}: {
  block: WorkBlock;
  tags: Map<string, Tag>;
  /** Raporun pencere aralıkları; verilirse uygulamaların altında pencereler de listelenir. */
  windows?: WindowSpan[];
}) {
  const edit = useEdit();
  const apps = useMemo(() => (windows ? blockWindows(windows, block.start, block.end) : null), [windows, block]);
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
      </div>
      <div>
        <div className="flex items-center justify-between gap-3">
          <span className="flex min-w-0 items-center gap-2">
            {block.topApps[0] && <AppIcon appId={block.topApps[0].appId} name={block.topApps[0].appName} size={20} />}
            <strong className="truncate text-sm">{block.topApps[0]?.appName ?? tag?.name ?? "Çalışma"}</strong>
          </span>
          <span className="text-sm font-semibold tabular">{formatDuration(block.activeSeconds)}</span>
        </div>
        <p className="mt-0.5 text-xs text-muted-foreground tabular">
          {formatTime(new Date(block.start))} – {formatTime(new Date(block.end))}
          {block.switches > 0 && ` · ${block.switches} uygulama geçişi`}
        </p>
      </div>
      {apps && apps.length > 0 ? (
        <BlockApps block={block} apps={apps} tags={tags} color={color} />
      ) : (
        <ul className="space-y-1.5">
          {block.topApps.map((a) => {
            const pct = block.activeSeconds ? Math.round((a.seconds / block.activeSeconds) * 100) : 0;
            return (
              <li key={a.appName} className="grid grid-cols-[34px_1fr_auto] items-center gap-2 text-xs">
                <span className="text-muted-foreground tabular">%{pct}</span>
                <span className="min-w-0">
                  <span className="flex min-w-0 items-center gap-1.5">
                    <AppIcon appId={a.appId} name={a.appName} size={14} />
                    <span className="truncate">{a.appName}</span>
                  </span>
                  <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                    <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
                  </span>
                </span>
                <span className="text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
              </li>
            );
          })}
        </ul>
      )}
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

/** Bir uygulamada gösterilen ilk pencere sayısı; gerisi "daha fazla" ile açılır. */
const FIRST_WINDOWS = 5;
/** Seçicide "kurallara bırak" (elle atamayı kaldır). */
const AUTO = "__otomatik__";

/**
 * Bloktaki uygulamalar ve her birinde zaman geçirilen pencereler: hangi işin yapıldığı (ve
 * hangi projeye yazıldığı) başlıktan anlaşılsın; pencere tek başına projeye atanabilir.
 */
function BlockApps({
  block,
  apps,
  tags,
  color,
}: {
  block: WorkBlock;
  apps: BlockApp[];
  tags: Map<string, Tag>;
  color: string;
}) {
  const total = apps.reduce((n, a) => n + a.seconds, 0);
  return (
    <div className="-mr-2 max-h-80 space-y-3 overflow-y-auto pr-2">
      {apps.map((a) => {
        const pct = total ? Math.round((a.seconds / total) * 100) : 0;
        return (
          <section key={a.appId}>
            <div className="grid grid-cols-[34px_1fr_auto] items-center gap-2 text-xs">
              <span className="text-muted-foreground tabular">%{pct}</span>
              <span className="min-w-0">
                <span className="flex min-w-0 items-center gap-1.5">
                  <AppIcon appId={a.appId} name={a.appName} size={16} />
                  <span className="truncate font-medium">{a.appName}</span>
                </span>
                <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                  <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
                </span>
              </span>
              <span className="text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
            </div>
            <AppWindows block={block} app={a} tags={tags} />
          </section>
        );
      })}
    </div>
  );
}

function AppWindows({ block, app, tags }: { block: WorkBlock; app: BlockApp; tags: Map<string, Tag> }) {
  const [all, setAll] = useState(false);
  const rows = useMemo(() => windowRows(app.windows, tags), [app.windows, tags]);
  // Yalnızca "Diğer" kalan uygulamada liste, üstteki süreyi tekrarlamaktan öteye geçmez.
  if (onlyOther(rows)) return null;
  const shown = all ? rows : rows.slice(0, FIRST_WINDOWS);
  const hidden = rows.length - shown.length;
  return (
    <ul className="mt-1.5 ml-[42px] space-y-0.5 border-l pl-2">
      {shown.map((r) =>
        r.kind === "item" ? (
          <WindowRow
            key={`${r.item.title}\u0000${r.item.projectId ?? ""}`}
            block={block}
            appId={app.appId}
            w={r.item}
            tags={tags}
          />
        ) : (
          <GroupRow key={r.key} row={r} tags={tags} />
        ),
      )}
      {(hidden > 0 || all) && rows.length > FIRST_WINDOWS && (
        <li>
          <button
            className="flex items-center gap-1 rounded py-0.5 text-[11px] text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
            aria-expanded={all}
            onClick={() => setAll(!all)}
          >
            <ChevronDown className={cn("size-3 transition-transform", all && "rotate-180")} />
            {all ? "Daha az göster" : `${hidden} satır daha`}
          </button>
        </li>
      )}
    </ul>
  );
}

/**
 * Bloktaki pencereler ayrıntı satırlarına: kısa olanlar aynı projede (projesizse aynı sitede)
 * birlikte anlamlıysa toplanır, gerisi "Diğer".
 */
function windowRows(windows: BlockWindow[], tags: Map<string, Tag>): DetailRow<BlockWindow>[] {
  return detailRows(
    windows,
    (w) => w.seconds,
    (w) => {
      const project = w.projectId && w.projectId !== NO_PROJECT ? tags.get(w.projectId) : undefined;
      if (project) return { key: `p:${project.id}`, label: project.name, projectId: project.id };
      if (w.domain) return { key: `d:${w.domain}`, label: w.domain, domain: w.domain };
      return null;
    },
  );
}

/** Kısa pencerelerin toplam satırı: adları tek tek gösterilmez, yalnızca ortak bağ ve süre. */
function GroupRow({ row, tags }: { row: Extract<DetailRow<BlockWindow>, { kind: "group" }>; tags: Map<string, Tag> }) {
  const project = row.bond?.projectId ? tags.get(row.bond.projectId) : undefined;
  return (
    <li className="flex items-center gap-2 rounded-md px-1.5 py-1 text-xs text-muted-foreground">
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        {project ? (
          <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} aria-hidden />
        ) : (
          row.bond?.domain && <SiteIcon domain={row.bond.domain} />
        )}
        <span className="truncate">{row.label}</span>
      </span>
      <span className="shrink-0 text-[11px] tabular">{formatDuration(row.seconds)}</span>
    </li>
  );
}

/** Pencere satırı: başlık, site, proje ve süre; "Ata" ile yalnızca bu pencerenin süresi atanır. */
function WindowRow({
  block,
  appId,
  w,
  tags,
}: {
  block: WorkBlock;
  appId: string;
  w: BlockWindow;
  tags: Map<string, Tag>;
}) {
  const edit = useEdit();
  const [open, setOpen] = useState(false);
  const project = w.projectId ? tags.get(w.projectId) : undefined;
  // Atama yalnızca zaman çizelgesine girecek kadar uzun (≥ 15 dk) atanmamış pencerede önerilir.
  const suggest = !project && w.seconds >= UNASSIGNED_MIN;
  const title = w.title || "(başlıksız)";

  async function assign(v: string) {
    if (!edit) return;
    const id = v === AUTO ? null : v;
    const name =
      id === null
        ? "kurallara bırakıldı"
        : id === NO_PROJECT
          ? "projesiz sayıldı"
          : `→ ${tags.get(id)?.name ?? "proje"}`;
    try {
      await undoable(api.assignWindow(block.start, block.end, appId, w.title, id), `“${short(title)}” ${name}`);
      setOpen(false);
      edit.onChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  return (
    <li className="group/w rounded-md px-1.5 py-1 hover:bg-accent/40">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className={cn("truncate text-xs", !w.title && "text-muted-foreground italic")} title={title}>
            {title}
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            {w.domain && (
              <span className="flex min-w-0 items-center gap-1 truncate">
                <SiteIcon domain={w.domain} />
                <span className="truncate">{w.domain}</span>
              </span>
            )}
            {w.domain && <span aria-hidden>·</span>}
            {project ? (
              <span className="flex min-w-0 items-center gap-1">
                <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} aria-hidden />
                <span className="truncate">{project.name}</span>
              </span>
            ) : (
              <span className={cn(suggest && "font-medium text-foreground")}>{UNASSIGNED}</span>
            )}
          </div>
        </div>
        <span className="shrink-0 pt-px text-[11px] text-muted-foreground tabular">{formatDuration(w.seconds)}</span>
        {edit && edit.projects.length > 0 && (
          <button
            className={cn(
              "flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-[11px] focus-visible:outline-2 focus-visible:outline-ring",
              suggest
                ? "border bg-background font-medium hover:bg-accent"
                : "text-muted-foreground opacity-0 group-hover/w:opacity-100 hover:bg-accent hover:text-foreground focus-visible:opacity-100",
              open && "opacity-100",
            )}
            aria-expanded={open}
            aria-label={`“${title}” penceresini projeye ata`}
            title="Bu blokta bu pencerede geçen süreyi projeye ata"
            onClick={() => setOpen(!open)}
          >
            <FolderInput className="size-3" aria-hidden />
            {suggest && "Ata"}
          </button>
        )}
      </div>
      {open && edit && (
        <ProjectSelect
          className="mt-1.5 w-full"
          value=""
          projects={edit.projects}
          placeholder="Projeye ata…"
          extra={[
            { value: NO_PROJECT, label: "Projesiz" },
            ...(w.projectId ? [{ value: AUTO, label: "Otomatik (kurallara göre)" }] : []),
          ]}
          aria-label={`“${title}” projesi`}
          onChange={assign}
        />
      )}
    </li>
  );
}

const short = (s: string) => (s.length > 40 ? `${s.slice(0, 39)}…` : s);

function blockTitle(b: WorkBlock, tags: Map<string, Tag>, lens: ColorLens) {
  const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
  const project = b.projectId ? tags.get(b.projectId) : undefined;
  return {
    // Proje merceğinde renk projeden; projesi olmayan blok taralı ve renksiz.
    color: lens === "project" ? (project ? tagColor(project) : null) : tagColor(tag),
    // Projeye atanmış blok proje adıyla görünür: atamanın sonucu takvimde hemen fark edilsin.
    title: project?.name ?? (lens === "project" ? UNASSIGNED : (tag?.name ?? b.topApps[0]?.appName ?? UNCATEGORIZED)),
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
  lens = "category",
  windows,
}: {
  b: WorkBlock;
  tags: Map<string, Tag>;
  top: number;
  height: number;
  windows?: WindowSpan[];
  /** Dar sütun (hafta): tek satırlık blokta süre yer kaplamasın, başlık okunsun. */
  narrow?: boolean;
  lens?: ColorLens;
}) {
  const { color: blockColor, title, apps } = blockTitle(b, tags, lens);
  const color = blockColor ?? "var(--c0)";
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
            background: blockColor ? `color-mix(in srgb, ${color} 22%, var(--card))` : `${HATCH}, var(--card)`,
          }}
          title={summary}
          aria-label={summary}
        >
          {label && (
            <span className={cn("flex h-full flex-col", full ? "py-1" : "justify-center")}>
              <span className="flex min-w-0 items-center gap-1.5">
                <AppIconStack apps={b.topApps} size={full ? 14 : 12} max={narrow ? 1 : 3} />
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
      <PopoverContent side="right" align="start" className={windows ? "w-96" : "w-72"}>
        <BlockDetails block={b} tags={tags} windows={windows} />
      </PopoverContent>
    </Popover>
  );
}

/**
 * Bilgisayardan uzakta geçen süre: taralı, renksiz blok. Tıklayınca aralık menüsü açılır
 * (projeye ya da kategoriye ata, elle kayıt olarak ekle); atanan kısım çalışma süresine girer.
 */
function IdleBlock({
  span,
  top,
  height,
  onSelect,
}: {
  span: IdleSpan;
  top: number;
  height: number;
  onSelect?: (start: number, end: number, x: number, y: number) => void;
}) {
  const a = new Date(span.start);
  const b = new Date(span.end);
  const time = `${formatTime(a)}–${formatTime(b)}`;
  const duration = formatDuration((+b - +a) / 1000);
  const tip = `Boşta · ${time} · ${duration}\nBilgisayardan uzakta geçen süre; çalışma süresine sayılmaz.`;
  return (
    <button
      type="button"
      disabled={!onSelect}
      className="absolute inset-x-0.5 overflow-hidden rounded-[5px] border border-dashed border-muted-foreground/35 px-1.5 text-left text-muted-foreground enabled:cursor-pointer enabled:hover:border-muted-foreground/60 enabled:hover:text-foreground"
      style={{
        top,
        height,
        background:
          "repeating-linear-gradient(135deg, color-mix(in srgb, var(--muted-foreground) 12%, transparent) 0 4px, transparent 4px 9px)",
      }}
      title={onSelect ? `${tip}\nTıkla: projeye ya da kategoriye ata, elle kayıt olarak ekle` : tip}
      aria-label={`Boşta ${time}`}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        const [x, y] = e.detail === 0 ? [r.right, r.top] : [e.clientX, e.clientY];
        onSelect?.(+a, +b, x, y);
      }}
    >
      {height >= LABEL_MIN_PX && (
        <span className={cn("flex h-full flex-col", height >= FULL_LABEL_PX ? "py-1" : "justify-center")}>
          <span className="flex items-center gap-1">
            <Coffee className="size-3 shrink-0" />
            <span className="truncate text-[11px] leading-tight font-medium">Boşta</span>
            {height < FULL_LABEL_PX && <span className="ml-auto shrink-0 text-[10px] tabular">{duration}</span>}
          </span>
          {height >= FULL_LABEL_PX && (
            <span className="truncate text-[10px] leading-tight tabular">{`${duration} · ${time}`}</span>
          )}
        </span>
      )}
    </button>
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

/** Gün takvimi: Oturumlar · Toplantılar (takvim bağlıysa) · Kategori şeridi. */
export function DayCalendar({
  from,
  blocks,
  idle = [],
  segments,
  windows,
  tags,
  meetings = null,
  onMeeting,
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
  lens = "category",
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
    () => hourRange(from, [...blocks, ...idle, ...segments, ...meetingSpans], hourPx),
    [from, blocks, idle, segments, meetingSpans, hourPx],
  );
  const top = topFn(+from, range);
  // 5 dk'lık dilim ancak rahat tıklanacak kadar yüksekse; yoksa 15 dk.
  const step = range.px >= 240 ? 5 : 15;
  const buckets = useMemo(() => categoryBuckets(segments, +from, step), [segments, from, step]);
  const showMeetings = meetings !== null;
  const grid = cn(
    "grid gap-x-2",
    showMeetings ? "grid-cols-[40px_minmax(0,1fr)_minmax(0,0.6fr)_14px]" : "grid-cols-[40px_minmax(0,1fr)_14px]",
  );
  const strip = showMeetings ? "col-start-4" : "col-start-3";

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
      </div>
      <div className={cn(grid, "pt-1.5")}>
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
              <Block key={block.start} b={block} tags={tags} lens={lens} windows={windows} top={t} height={height} />
            ),
          )}
          <Preview range={preview} top={top} />
          <NowLine day={from} range={range} />
        </Column>
        {showMeetings && (
          <Column range={range} className="col-start-3 row-start-1">
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

/** Hafta takvimi: her gün bir sütun, bloklar kategori (ya da proje) renginde. */
export function WeekCalendar({
  from,
  blocks,
  idle = [],
  dayTotals,
  windows,
  tags,
  onSelectDay,
  onEmpty,
  onRange,
  preview,
  hourPx = HOUR_PX,
  lens = "category",
}: {
  from: Date;
  blocks: WorkBlock[];
  idle?: IdleSpan[];
  dayTotals: number[];
  windows?: WindowSpan[];
  tags: Map<string, Tag>;
  onSelectDay: (iso: string) => void;
  onEmpty?: (start: number, end: number) => void;
  onRange?: (start: number, end: number, x: number, y: number) => void;
  preview?: [number, number] | null;
  hourPx?: number;
  lens?: ColorLens;
}) {
  const range = useMemo(() => hourRange(from, [...blocks, ...idle], hourPx, 7), [from, blocks, idle, hourPx]);
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
              {placeSessions(
                blocks.filter((b) => +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd),
                idle.filter((s) => +new Date(s.start) >= dayStart && +new Date(s.start) < dayEnd),
                top,
                range.px,
              ).map(({ block, idle: span, top: t, height }) =>
                span ? (
                  <IdleBlock key={`idle-${span.start}`} span={span} onSelect={onRange} top={t} height={height} />
                ) : (
                  <Block
                    key={block.start}
                    b={block}
                    tags={tags}
                    narrow
                    lens={lens}
                    windows={windows}
                    top={t}
                    height={height}
                  />
                ),
              )}
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

/** Aralığın bundan azı takip edildiyse şeritte boş kalır. */
const BUCKET_MIN_SHARE = 1 / 3;

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
    if (total < step * BUCKET_MIN_SHARE) return;
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
