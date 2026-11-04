import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { Check, Combine, Scissors, Trash2 } from "lucide-react";
import type { Tag, WindowSpan } from "../../api";
import { api, formatDuration, NO_PROJECT } from "../../api";
import { addDays, formatTime, fromWallMs } from "../../lib/dates";
import { friendlyError, toast, undoable, useChanged } from "../../lib/feedback";
import {
  type Slot,
  assignRanges,
  confirm,
  draftSlots,
  fromApi,
  mergeWith,
  paint,
  remove,
  resize,
  snap,
  split,
  toApi,
  update,
} from "../../lib/plan";
import { quickProjects, rememberRecent } from "../../lib/projectOptions";
import { UNASSIGNED, tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { blockWindows } from "../../lib/blockWindows";
import { ProjectSelect } from "../ProjectSelect";
import { useEdit } from "../SessionEdit";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { Column, DRAG_MIN_PX, FULL_LABEL_PX, HATCH, HOUR_MS, LABEL_MIN_PX, type Range } from "./grid";
import { topWindow } from "./windowLanes";

const iso = (t: number) => new Date(t).toISOString();
const time = (t: number) => formatTime(new Date(t));

/**
 * Günün planı: onaylı bloklar sunucudan, taslaklar pencere kaydından. `enabled` değilse hiçbir
 * şey yüklenmez (eski görünüm).
 */
export function usePlan(from: Date, windows: WindowSpan[], enabled: boolean) {
  const edit = useEdit();
  const dayStart = +from;
  const dayEnd = +addDays(from, 1);
  const [confirmed, setConfirmed] = useState<Slot[] | null>(null);
  const load = useCallback(() => {
    if (!enabled) return;
    api.getPlan(iso(dayStart), iso(dayEnd)).then(
      (list) => setConfirmed(list.map(fromApi)),
      (e) => toast(friendlyError(e), { tone: "error" }),
    );
  }, [dayStart, dayEnd, enabled]);
  useEffect(() => {
    setConfirmed(null);
    load();
  }, [load]);
  // Geri alma ve başka sayfalardaki değişiklikler.
  useChanged(load);
  const drafts = useMemo(
    () => (confirmed ? draftSlots(windows, dayStart, dayEnd, Date.now(), confirmed) : []),
    [windows, dayStart, dayEnd, confirmed],
  );
  /** Planı kaydeder (iyimser); oturumlar da değişen aralıklarda projeye geçer. */
  const save = useCallback(
    async (next: Slot[], message: string) => {
      if (!confirmed) return;
      const clipped = next
        .map((b) => ({ ...b, start: Math.max(b.start, dayStart), end: Math.min(b.end, dayEnd) }))
        .filter((b) => b.end > b.start && !b.draft);
      const assign = assignRanges(confirmed, clipped).map((r) => ({
        start: iso(r.start),
        end: iso(r.end),
        projectId: r.projectId,
      }));
      const before = confirmed;
      setConfirmed(clipped);
      try {
        await undoable(api.savePlan(iso(dayStart), iso(dayEnd), clipped.map(toApi), assign), message);
        load();
        if (assign.length) edit?.onChanged();
      } catch (e) {
        setConfirmed(before);
        toast(friendlyError(e), { tone: "error" });
      }
    },
    [confirmed, dayStart, dayEnd, load, edit],
  );

  return { confirmed, drafts, save, dayStart, dayEnd };
}

/**
 * Gün planı sütunu: onaylı bloklar (dolu) ve taslaklar (kesikli, çizgili). Bloğa tıklayınca
 * projesi, notu ve içindeki pencereler; kenarı sürüklenince yalnızca kenarı (bitişik komşuyla
 * ortak sınır) oynar; çift tıklayınca o noktadan bölünür; sütunda sürükleyince aralık tek
 * seferde bir projeye boyanır. Taslağa yapılan her düzenleme onu onaylı bloğa çevirir.
 */
export function PlanColumn({
  range,
  top,
  windows,
  tags,
  plan,
  className,
  children,
  onDrag,
}: {
  /** Sütunda sürükleyerek aralık seçildi (ipucunu kapatmak için). */
  onDrag?: () => void;
  range: Range;
  top: (t: number) => number;
  windows: WindowSpan[];
  tags: Map<string, Tag>;
  plan: ReturnType<typeof usePlan>;
  className?: string;
  children?: React.ReactNode;
}) {
  const edit = useEdit();
  const projects = useMemo(() => edit?.projects ?? [], [edit]);
  const { confirmed, drafts, save, dayStart, dayEnd } = plan;
  const [paintSel, setPaintSel] = useState<{ a: number; b: number; x: number; y: number } | null>(null);
  const all = useMemo(() => [...(confirmed ?? []), ...drafts].sort((a, b) => a.start - b.start), [confirmed, drafts]);

  /**
   * Düzenlemeden önce ilgili taslakları onaylı bloğa çevirir: onaylı liste ve taslakların yeni
   * kimlikleri döner.
   */
  const materialize = (ids: string[]): [Slot[], Map<string, string>] => {
    const map = new Map<string, string>();
    const list = (confirmed ?? []).slice();
    for (const d of drafts) {
      if (!ids.includes(d.id)) continue;
      const c = confirm(d);
      map.set(d.id, c.id);
      list.push(c);
    }
    return [list, map];
  };
  const idOf = (map: Map<string, string>, id: string) => map.get(id) ?? id;

  const actions: PlanActions = {
    setProject: (b, projectId) => {
      const [list, map] = materialize([b.id]);
      if (projectId && projectId !== NO_PROJECT) rememberRecent(projectId);
      void save(update(list, idOf(map, b.id), { projectId }), `${projectName(projectId, tags)} olarak kaydedildi`);
    },
    setNote: (b, note) => {
      const [list, map] = materialize([b.id]);
      void save(update(list, idOf(map, b.id), { note }), "Not kaydedildi");
    },
    approve: (b) => {
      const [list] = materialize([b.id]);
      void save(list, "Blok onaylandı");
    },
    split: (b, t) => {
      const [list, map] = materialize([b.id]);
      void save(split(list, idOf(map, b.id), snap(t)), `${time(snap(t))}'de bölündü`);
    },
    merge: (b, dir) => {
      const i = all.findIndex((x) => x.id === b.id);
      const other = all[dir === "prev" ? i - 1 : i + 1];
      if (!other) return;
      const [list, map] = materialize([b.id, other.id]);
      void save(mergeWith(list, idOf(map, b.id), dir), "Bloklar birleşti");
    },
    remove: (b) => void save(remove(confirmed ?? [], b.id), "Blok kaldırıldı; yerine taslak önerilir"),
    resize: (b, edge, t) => {
      const [list, map] = materialize([b.id]);
      const next = resize(list, idOf(map, b.id), edge, snap(t), Math.min(Date.now(), dayEnd));
      void save(next, "Blok değişti");
    },
  };

  const neighbors = (b: Slot) => {
    const i = all.findIndex((x) => x.id === b.id);
    return { prev: i > 0, next: i >= 0 && i < all.length - 1 };
  };

  return (
    <>
      <Column
        range={range}
        className={className}
        onRange={(a, b, x, y) => {
          onDrag?.();
          setPaintSel({ a: fromWallMs(a, dayStart), b: fromWallMs(b, dayStart), x, y });
        }}
      >
        {all.map((b) => (
          <PlanBlockView
            key={b.id}
            b={b}
            tags={tags}
            projects={projects}
            windows={windows}
            top={top}
            hourPx={range.px}
            actions={actions}
            neighbors={neighbors(b)}
          />
        ))}
        {children}
      </Column>
      {paintSel && (
        <PaintMenu
          sel={paintSel}
          projects={projects}
          onClose={() => setPaintSel(null)}
          onPick={(projectId) => {
            const { a, b } = paintSel;
            setPaintSel(null);
            const end = Math.min(b, Date.now());
            if (end - a < 5 * 60_000) return;
            if (projectId && projectId !== NO_PROJECT) rememberRecent(projectId);
            void save(
              paint(confirmed ?? [], a, end, projectId),
              `${time(a)}–${time(end)} ${projectName(projectId, tags)} olarak kaydedildi`,
            );
          }}
        />
      )}
    </>
  );
}

/** Daha önce zaman çizelgesine yazılan açıklamalar (blok adı önerileri); açılışta bir kez okunur. */
let pastNames: Promise<string[]> | null = null;

function usePastNames(): string[] {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    pastNames ??= api.timesheetDetails().catch(() => {
      pastNames = null;
      return [];
    });
    let live = true;
    void pastNames.then((n) => live && setNames(n));
    return () => {
      live = false;
    };
  }, []);
  return names;
}

/** Taslakların hepsini onaylar. */
export function approveAll(plan: ReturnType<typeof usePlan>): Slot[] {
  return [...(plan.confirmed ?? []), ...plan.drafts.map(confirm)];
}

type PlanActions = {
  setProject: (b: Slot, projectId: string | null) => void;
  setNote: (b: Slot, note: string) => void;
  approve: (b: Slot) => void;
  split: (b: Slot, t: number) => void;
  merge: (b: Slot, dir: "prev" | "next") => void;
  remove: (b: Slot) => void;
  resize: (b: Slot, edge: "start" | "end", t: number) => void;
};

function projectName(projectId: string | null, tags: Map<string, Tag>) {
  if (projectId === NO_PROJECT) return "Projesiz";
  return (projectId && tags.get(projectId)?.name) || UNASSIGNED;
}

/** Bloğun başlığı: verilen adı, yoksa projesi, o da yoksa içinde en çok süren pencere. */
function titleOf(b: Slot, tags: Map<string, Tag>, windows: WindowSpan[]) {
  if (b.note.trim()) return b.note.trim();
  if (b.projectId === NO_PROJECT) return "Projesiz";
  if (b.projectId) return tags.get(b.projectId)?.name ?? "Silinen proje";
  return topWindow(windows, b.start, b.end)?.title ?? UNASSIGNED;
}

function PlanBlockView({
  b,
  tags,
  projects,
  windows,
  top,
  hourPx,
  actions,
  neighbors,
}: {
  b: Slot;
  tags: Map<string, Tag>;
  projects: Tag[];
  windows: WindowSpan[];
  top: (t: number) => number;
  hourPx: number;
  actions: PlanActions;
  neighbors: { prev: boolean; next: boolean };
}) {
  const [open, setOpen] = useState(false);
  // Sürüklenen kenar ve şu anki konumu (ms).
  const [drag, setDrag] = useState<{ edge: "start" | "end"; y0: number; at: number } | null>(null);
  const start = drag?.edge === "start" ? drag.at : b.start;
  const end = drag?.edge === "end" ? drag.at : b.end;
  const y = top(start);
  const height = Math.max(6, top(end) - y - 2);
  const project = b.projectId && b.projectId !== NO_PROJECT ? tags.get(b.projectId) : undefined;
  const color = project ? tagColor(project) : "var(--muted-foreground)";
  const title = titleOf(b, tags, windows);
  const span = formatDuration((end - start) / 1000);
  const summary = `${title} · ${time(start)}–${time(end)} · ${span}${b.draft ? " · taslak" : ""}`;
  const full = height >= FULL_LABEL_PX;
  const label = height >= LABEL_MIN_PX;

  const handle = (edge: "start" | "end") => (
    <span
      aria-hidden
      className={cn(
        "group/h absolute inset-x-1 z-[5] flex h-2 cursor-ns-resize justify-center",
        edge === "end" && "items-end",
      )}
      style={{ top: edge === "start" ? y - 2 : y + height - 6 }}
      title="Sürükle: yalnızca bu kenar oynar (bitişik bloğun kenarı da birlikte kayar)"
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.stopPropagation();
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        setDrag({ edge, y0: e.clientY, at: edge === "start" ? b.start : b.end });
      }}
      onPointerMove={(e) => {
        if (!drag) return;
        const base = drag.edge === "start" ? b.start : b.end;
        const dy = e.clientY - drag.y0;
        const at = Math.abs(dy) < DRAG_MIN_PX ? base : snap(base + (dy / hourPx) * HOUR_MS);
        const clamped = drag.edge === "start" ? Math.min(at, b.end - 5 * 60_000) : Math.max(at, b.start + 5 * 60_000);
        if (clamped !== drag.at) setDrag({ ...drag, at: Math.min(clamped, Date.now()) });
      }}
      onPointerUp={() => {
        if (!drag) return;
        setDrag(null);
        const base = drag.edge === "start" ? b.start : b.end;
        if (drag.at !== base) actions.resize(b, drag.edge, drag.at);
      }}
      onPointerCancel={() => setDrag(null)}
    >
      <i className="mx-auto h-[3px] w-6 rounded-full bg-foreground/40 opacity-0 transition-opacity group-hover/h:opacity-100" />
    </span>
  );

  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            data-plan={b.draft ? "draft" : "block"}
            className={cn(
              "absolute inset-x-0.5 overflow-hidden rounded-[5px] px-1.5 text-left transition-[filter] hover:brightness-95 focus-visible:outline-2 focus-visible:outline-ring data-[state=open]:ring-2 data-[state=open]:ring-[var(--cat)] dark:hover:brightness-125",
              b.draft ? "border border-dashed" : "border-l-[3px]",
              drag && "ring-2 ring-[var(--cat)]",
            )}
            style={{
              top: y,
              height,
              ["--cat" as string]: color,
              borderColor: b.draft ? `color-mix(in srgb, ${color} 60%, transparent)` : color,
              background: b.draft
                ? `repeating-linear-gradient(135deg, color-mix(in srgb, ${color} 14%, transparent) 0 6px, transparent 6px 12px), var(--card)`
                : project
                  ? `color-mix(in srgb, ${color} 22%, var(--card))`
                  : `${HATCH}, var(--card)`,
            }}
            title={`${summary}\nTıkla: proje, not, böl, birleştir · Çift tıkla: buradan böl · Kenarı sürükle: boyut`}
            aria-label={label ? undefined : summary}
            onDoubleClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              const t = b.start + ((e.clientY - r.top) / r.height) * (b.end - b.start);
              setOpen(false);
              actions.split(b, t);
            }}
          >
            {label && (
              <span className={cn("relative flex h-full flex-col", full ? "py-1" : "justify-center")}>
                <span className="flex min-w-0 items-center gap-1.5">
                  <span className="truncate text-[11px] leading-tight font-semibold">{title}</span>
                  {b.draft && (
                    <span className="shrink-0 rounded-full border border-dashed px-1 text-[9px] leading-tight text-muted-foreground">
                      taslak
                    </span>
                  )}
                  {!full && <span className="ml-auto shrink-0 text-[10px] text-muted-foreground tabular">{span}</span>}
                </span>
                {full && (
                  <span className="truncate text-[10px] leading-tight text-muted-foreground tabular">
                    {b.note.trim() && b.projectId && `${projectName(b.projectId, tags)} · `}
                    {time(start)}–{time(end)} · {span}
                  </span>
                )}
              </span>
            )}
          </button>
        </PopoverTrigger>
        <PopoverContent side="right" align="start" className="w-80">
          <PlanDetails
            b={b}
            tags={tags}
            projects={projects}
            windows={windows}
            actions={actions}
            neighbors={neighbors}
            onDone={() => setOpen(false)}
          />
        </PopoverContent>
      </Popover>
      {handle("start")}
      {handle("end")}
    </>
  );
}

function PlanDetails({
  b,
  tags,
  projects,
  windows,
  actions,
  neighbors,
  onDone,
}: {
  b: Slot;
  tags: Map<string, Tag>;
  projects: Tag[];
  windows: WindowSpan[];
  actions: PlanActions;
  neighbors: { prev: boolean; next: boolean };
  onDone: () => void;
}) {
  const [note, setNote] = useState(b.note);
  useEffect(() => setNote(b.note), [b.note]);
  const [quick] = useState(() => quickProjects(projects));
  const apps = useMemo(() => blockWindows(windows, iso(b.start), iso(b.end)).slice(0, 4), [windows, b.start, b.end]);
  // Ad önerileri (zaman çizelgesindeki gibi): önce bloğun pencere başlıkları, sonra daha önce
  // yazılan açıklamalar. Yazdıkça süzülür.
  const past = usePastNames();
  const listId = useId();
  const suggestions = useMemo(() => {
    const titles = blockWindows(windows, iso(b.start), iso(b.end)).flatMap((a) => a.windows.map((w) => w.title.trim()));
    return [...new Set([...titles, ...past.map((d) => d.trim())].filter(Boolean))].slice(0, 200);
  }, [windows, b.start, b.end, past]);
  const run = (f: () => void) => {
    onDone();
    f();
  };
  return (
    <div className="space-y-3">
      <div>
        <div className="flex items-center justify-between gap-3">
          <strong className="truncate text-sm">{titleOf(b, tags, windows)}</strong>
          <span className="text-sm font-semibold tabular">{formatDuration((b.end - b.start) / 1000)}</span>
        </div>
        <p className="mt-0.5 text-xs text-muted-foreground tabular">
          {time(b.start)} – {time(b.end)}
          {b.draft && " · taslak: pencere kaydından önerildi"}
        </p>
      </div>
      <label className="block space-y-1">
        <span className="block text-[11px] text-muted-foreground">Blok adı · zaman çizelgesinde açıklama olur</span>
        <input
          value={note}
          placeholder={titleOf({ ...b, note: "" }, tags, windows)}
          aria-label="Blok adı"
          list={listId}
          autoComplete="off"
          className="h-8 w-full rounded-md border bg-transparent px-2 text-sm"
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          onBlur={() => note.trim() !== b.note && actions.setNote(b, note.trim())}
        />
        <datalist id={listId}>
          {suggestions.map((d) => (
            <option key={d} value={d} />
          ))}
        </datalist>
      </label>
      <div className="grid gap-1" role="group" aria-label="Proje">
        {[
          ...quick,
          ...(b.projectId && !quick.some((p) => p.id === b.projectId)
            ? projects.filter((p) => p.id === b.projectId)
            : []),
        ].map((p) => (
          <button
            key={p.id}
            type="button"
            aria-pressed={b.projectId === p.id}
            className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-accent aria-pressed:bg-accent aria-pressed:font-semibold"
            onClick={() => run(() => actions.setProject(b, p.id))}
          >
            <i className="size-2.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
            <span className="truncate">{p.name}</span>
            {b.projectId === p.id && <Check className="ml-auto size-3.5" />}
          </button>
        ))}
        <ProjectSelect
          value=""
          projects={projects}
          placeholder="Başka proje…"
          extra={[{ value: NO_PROJECT, label: "Projesiz (iş değil)" }]}
          className="w-full"
          onChange={(v) => run(() => actions.setProject(b, v))}
        />
      </div>
      {apps.length > 0 && (
        <ul className="space-y-0.5 text-xs text-muted-foreground">
          {apps.map((a) => (
            <li key={a.appId} className="flex justify-between gap-2">
              <span className="truncate">{a.windows[0]?.title || a.appName}</span>
              <span className="shrink-0 tabular">{formatDuration(a.seconds)}</span>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap gap-1.5">
        {b.draft && (
          <Button size="sm" onClick={() => run(() => actions.approve(b))}>
            <Check /> Onayla
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          title="Ortadan böl (çift tıklayarak istediğin yerden de bölebilirsin)"
          onClick={() => run(() => actions.split(b, (b.start + b.end) / 2))}
        >
          <Scissors /> Böl
        </Button>
        {neighbors.prev && (
          <Button size="sm" variant="outline" onClick={() => run(() => actions.merge(b, "prev"))}>
            <Combine /> Üsttekiyle
          </Button>
        )}
        {neighbors.next && (
          <Button size="sm" variant="outline" onClick={() => run(() => actions.merge(b, "next"))}>
            <Combine /> Alttakiyle
          </Button>
        )}
        {!b.draft && (
          <Button
            size="icon-sm"
            variant="ghost"
            className="text-muted-foreground hover:text-destructive"
            aria-label="Bloğu kaldır"
            title="Bloğu kaldır (yerine taslak önerilir; kayıtlar silinmez)"
            onClick={() => run(() => actions.remove(b))}
          >
            <Trash2 />
          </Button>
        )}
      </div>
    </div>
  );
}

/** Sürükleyerek seçilen aralığın proje menüsü (tek tık ya da 1–4 tuşları). */
function PaintMenu({
  sel,
  projects,
  onPick,
  onClose,
}: {
  sel: { a: number; b: number; x: number; y: number };
  projects: Tag[];
  onPick: (projectId: string | null) => void;
  onClose: () => void;
}) {
  const [quick] = useState(() => quickProjects(projects));
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return onClose();
      if ((e.target as HTMLElement | null)?.closest("input, textarea, select")) return;
      const n = Number(e.key);
      if (!(n >= 1 && n <= quick.length)) return;
      e.preventDefault();
      e.stopPropagation();
      onPick(quick[n - 1].id);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [quick, onPick, onClose]);
  const width = 248;
  return (
    <>
      <div className="fixed inset-0 z-40" onPointerDown={onClose} />
      <div
        role="dialog"
        aria-label="Aralığı ata"
        className="fixed z-50 space-y-2 rounded-lg border bg-popover p-2 text-popover-foreground shadow-lg"
        style={{
          left: Math.min(sel.x + 12, window.innerWidth - width - 12),
          top: Math.min(sel.y, window.innerHeight - 260),
          width,
        }}
      >
        <p className="px-2 pt-1 text-xs text-muted-foreground tabular">
          {time(sel.a)} – {time(Math.min(sel.b, Date.now()))} · tek blok olarak kaydet
        </p>
        <div className="grid gap-1" role="group" aria-label="Projeye ata">
          {quick.map((p, i) => (
            <button
              key={p.id}
              type="button"
              className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent"
              onClick={() => onPick(p.id)}
            >
              <i className="size-2.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
              <span className="truncate">{p.name}</span>
              <kbd className="ml-auto shrink-0 rounded border px-1 font-sans text-[10px] text-muted-foreground">
                {i + 1}
              </kbd>
            </button>
          ))}
        </div>
        <ProjectSelect
          value=""
          projects={projects}
          placeholder={quick.length ? "Başka proje…" : "Projeye ata…"}
          extra={[{ value: NO_PROJECT, label: "Projesiz (iş değil)" }]}
          className="w-full"
          onChange={(v) => onPick(v)}
        />
      </div>
    </>
  );
}
