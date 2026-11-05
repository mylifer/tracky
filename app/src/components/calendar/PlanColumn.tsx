import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { Check, Combine, Scissors, Trash2 } from "lucide-react";
import type { CalendarMeeting, Tag, WindowSpan } from "../../api";
import { api, formatDuration, NO_PROJECT } from "../../api";
import { addDays, formatTime, fromWallMs, isoDate } from "../../lib/dates";
import { friendlyError, toast, undoable, useChanged } from "../../lib/feedback";
import { buildModel, relabel } from "../../lib/learn";
import {
  PLAN_DEFAULTS,
  type Slot,
  confirm,
  draftSlots,
  draw,
  fromApi,
  meetingSlots,
  mergeWith,
  remove,
  resize,
  snap,
  split,
  subtract,
  toApi,
  update,
  visibleSuggestions,
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
/** Öneri modelinin baktığı geçmiş (gün). */
const HISTORY_DAYS = 60;

/** Önerilerden verilen aralıkları çıkarır (5 dakikadan kısa kalan parçalar düşer). */
function cutOut(list: Slot[], spans: { start: number; end: number }[]): Slot[] {
  return list.flatMap((d) =>
    subtract(d.start, d.end, spans)
      .filter(([x, y]) => y - x >= 5 * 60_000)
      .map(([x, y]) => ({ ...d, start: x, end: y })),
  );
}

/** Bu cihazda saklanan liste (günün önerileri, silinen etkinlik önerileri). */
function useStored<T>(key: string, empty: T): [T, (v: T) => void] {
  const read = useCallback((): T => {
    try {
      const v = localStorage.getItem(key);
      return v ? (JSON.parse(v) as T) : empty;
    } catch {
      return empty;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const [value, setValue] = useState<T>(read);
  useEffect(() => setValue(read()), [read]);
  return [
    value,
    (v) => {
      setValue(v);
      try {
        localStorage.setItem(key, JSON.stringify(v));
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}

/**
 * Günün planı (docs/gun-plani.md): onaylı bloklar sunucudan; öneriler takvim etkinliklerinden
 * (her zaman) ve "Öner" ile bilgisayar kaydından (bu cihazda saklanır, kendiliğinden yenilenmez).
 * `enabled` değilse hiçbir şey yüklenmez (eski görünüm).
 */
export function usePlan(from: Date, windows: WindowSpan[], meetings: CalendarMeeting[] | null, enabled: boolean) {
  const edit = useEdit();
  const dayStart = +from;
  const dayEnd = +addDays(from, 1);
  const day = isoDate(from);
  const [confirmed, setConfirmed] = useState<Slot[] | null>(null);
  const [stored, setStored] = useStored<Slot[]>(`kum.plan.suggestions.${day}`, []);
  const [dismissed, setDismissed] = useStored<string[]>(`kum.plan.dismissedMeetings.${day}`, []);
  const [suggesting, setSuggesting] = useState(false);
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

  const meetingDrafts = useMemo(
    () => (confirmed ? meetingSlots(meetings ?? [], dayStart, dayEnd, confirmed, new Set(dismissed)) : []),
    [meetings, dayStart, dayEnd, confirmed, dismissed],
  );
  const drafts = useMemo(
    () =>
      confirmed
        ? [...meetingDrafts, ...visibleSuggestions(stored, confirmed, meetingDrafts)].sort((a, b) => a.start - b.start)
        : [],
    [confirmed, stored, meetingDrafts],
  );

  /** Planı kaydeder (iyimser). Plan ham kayda yazmaz (kural 9). */
  const save = useCallback(
    async (next: Slot[], message: string) => {
      if (!confirmed) return;
      const clipped = next
        .map((b) => ({ ...b, start: Math.max(b.start, dayStart), end: Math.min(b.end, dayEnd) }))
        .filter((b) => b.end > b.start && !b.draft);
      const before = confirmed;
      setConfirmed(clipped);
      // Onaylanan saatin önerisi bir daha görünmez: blok sonra küçülse ya da silinse de orası
      // boş kalır (kural 4).
      setStored(cutOut(stored, clipped));
      const taken = meetingDrafts.filter((m) => clipped.some((c) => c.start < m.end && m.start < c.end));
      if (taken.length) setDismissed([...dismissed, ...taken.map((m) => m.meeting!)]);
      try {
        await undoable(api.savePlan(iso(dayStart), iso(dayEnd), clipped.map(toApi), []), message);
        load();
        // Zaman çizelgesi sütunu onaylı bloklardan gelir: yenilensin.
        edit?.onChanged();
      } catch (e) {
        setConfirmed(before);
        toast(friendlyError(e), { tone: "error" });
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [confirmed, dayStart, dayEnd, load, edit, stored, dismissed, meetingDrafts],
  );

  /** Öneriyi siler (V3): bilgisayar önerisi "Öner"e yeniden basılana dek, etkinlik o gün gelmez. */
  const dismiss = (d: Slot) => {
    if (d.meeting) setDismissed([...dismissed, d.meeting]);
    else setStored(cutOut(stored, [d]));
  };

  /**
   * "Öner" (kural 3): geçmişte onaylanan bloklardan öğrenen modelle bilgisayar kaydından öneriler
   * hazırlar ve saklar. Onaylı blokların ve takvim etkinliklerinin saatine öneri girmez.
   */
  const suggest = async () => {
    if (!confirmed || suggesting) return;
    setSuggesting(true);
    try {
      const examples = await api.getPlanExamples(HISTORY_DAYS).catch(() => []);
      const model = buildModel(examples);
      const { windows: labeled, guessed, names } = relabel(model, windows);
      const list = draftSlots(labeled, dayStart, dayEnd, Date.now(), [...confirmed, ...meetingDrafts], {
        ...PLAN_DEFAULTS,
        names,
      });
      setStored(list);
      toast(
        list.length === 0
          ? "Önerilecek yeni süre yok"
          : guessed > 0
            ? `${list.length} öneri hazırlandı (geçmişinden öğrenildi)`
            : `${list.length} öneri hazırlandı (henüz öğrenecek geçmiş yok: basit öneriler)`,
      );
    } finally {
      setSuggesting(false);
    }
  };

  return { confirmed, drafts, save, dismiss, suggest, suggesting, dayStart, dayEnd };
}

export type Plan = ReturnType<typeof usePlan>;

/** Önerilerin hepsini onaylar (V2). */
export function approveAll(plan: Plan): Slot[] {
  return [...(plan.confirmed ?? []), ...plan.drafts.map(confirm)];
}

/**
 * Gün planı sütunu (docs/gun-plani.md): onaylı bloklar dolu, öneriler kesik çizgili. Bloğa
 * tıklayınca proje, ad, içindeki pencereler, böl ve birleştir; kenarı sürüklenince yalnızca o
 * blok değişir; sütunda sürükleyince boş alana yeni blok çizilir. Öneriye yapılan her düzenleme
 * onu onaylar.
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
  plan: Plan;
  className?: string;
  children?: React.ReactNode;
}) {
  const edit = useEdit();
  const projects = useMemo(() => edit?.projects ?? [], [edit]);
  const { confirmed, drafts, save, dismiss, dayStart, dayEnd } = plan;
  const [paintSel, setPaintSel] = useState<{ a: number; b: number; x: number; y: number } | null>(null);
  // Menüden "Böl" seçilen blok: bir sonraki tıklama bölme yeridir (kural 8).
  const [splitting, setSplitting] = useState<string | null>(null);
  useEffect(() => {
    if (!splitting) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setSplitting(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [splitting]);
  const all = useMemo(() => [...(confirmed ?? []), ...drafts].sort((a, b) => a.start - b.start), [confirmed, drafts]);

  /** Düzenlemeden önce ilgili önerileri onaylı bloğa çevirir: onaylı liste ve yeni kimlikler. */
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
      void save(update(list, idOf(map, b.id), { note }), "Ad kaydedildi");
    },
    approve: (b) => {
      const [list] = materialize([b.id]);
      void save(list, "Blok onaylandı");
    },
    startSplit: (b) => setSplitting(b.id),
    split: (b, t) => {
      setSplitting(null);
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
    remove: (b) => {
      if (b.draft) dismiss(b);
      else void save(remove(confirmed ?? [], b.id), "Blok kaldırıldı; yeri boş kalır");
    },
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
            splitting={splitting === b.id}
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
            if (projectId && projectId !== NO_PROJECT) rememberRecent(projectId);
            const next = draw(confirmed ?? [], a, end, projectId);
            if (next === confirmed) return toast("Burada boş yer yok: yeni blok yalnızca boş alana çizilir");
            void save(next, `${projectName(projectId, tags)} bloğu eklendi`);
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

type PlanActions = {
  setProject: (b: Slot, projectId: string | null) => void;
  setNote: (b: Slot, note: string) => void;
  approve: (b: Slot) => void;
  startSplit: (b: Slot) => void;
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
  splitting,
}: {
  b: Slot;
  tags: Map<string, Tag>;
  projects: Tag[];
  windows: WindowSpan[];
  top: (t: number) => number;
  hourPx: number;
  actions: PlanActions;
  neighbors: { prev: boolean; next: boolean };
  /** "Böl" seçildi: bloğa tıklanan yer bölme yeridir. */
  splitting: boolean;
}) {
  const [open, setOpen] = useState(false);
  // Bölme modunda imlecin gösterdiği bölme yeri (ms).
  const [cut, setCut] = useState<number | null>(null);
  useEffect(() => setCut(null), [splitting]);
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
  const summary = `${title} · ${time(start)}–${time(end)} · ${span}${b.draft ? " · öneri" : ""}`;
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
      title="Sürükle: yalnızca bu blok değişir, yandaki bloğun kenarında durur"
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
            title={`${summary}\nTıkla: proje, ad, onayla, böl, birleştir · Kenarı sürükle: boyut`}
            aria-label={label ? undefined : summary}
          >
            {label && (
              <span className={cn("relative flex h-full flex-col", full ? "py-1" : "justify-center")}>
                <span className="flex min-w-0 items-center gap-1.5">
                  <span className="truncate text-[11px] leading-tight font-semibold">{title}</span>
                  {b.draft && (
                    <span className="shrink-0 rounded-full border border-dashed px-1 text-[9px] leading-tight text-muted-foreground">
                      öneri
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
      {splitting && (
        <div
          role="button"
          tabIndex={0}
          aria-label={`${title}: bölmek için tıkla`}
          data-plan-split
          className="absolute inset-x-0.5 z-[6] cursor-row-resize rounded-[5px] ring-2 ring-[var(--cat)]"
          style={{ top: y, height, ["--cat" as string]: color }}
          title="Bölmek istediğin yere tıkla (Esc: vazgeç)"
          onPointerMove={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const t = snap(b.start + ((e.clientY - r.top) / r.height) * (b.end - b.start));
            setCut(t > b.start && t < b.end ? t : null);
          }}
          onPointerLeave={() => setCut(null)}
          // Sütunun sürükleyerek çizmesi başlamasın.
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            actions.split(b, b.start + ((e.clientY - r.top) / r.height) * (b.end - b.start));
          }}
          onKeyDown={(e) => e.key === "Enter" && actions.split(b, (b.start + b.end) / 2)}
        >
          {cut !== null && (
            <span
              className="pointer-events-none absolute inset-x-0 border-t-2 border-dashed border-foreground"
              style={{ top: top(cut) - y }}
            >
              <span className="absolute -top-2.5 right-1 rounded bg-foreground px-1 text-[9px] text-background tabular">
                {time(cut)}
              </span>
            </span>
          )}
        </div>
      )}
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
          {b.draft && (b.meeting ? " · öneri: takvim etkinliği" : " · öneri: bilgisayar kaydından")}
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
          title="Sonra blokta bölmek istediğin yere tıkla"
          onClick={() => run(() => actions.startSplit(b))}
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
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-muted-foreground hover:text-destructive"
          aria-label={b.draft ? "Öneriyi sil" : "Bloğu kaldır"}
          title={
            b.draft
              ? b.meeting
                ? "Öneriyi sil (bu etkinlik bugün bir daha önerilmez)"
                : 'Öneriyi sil ("Öner"e yeniden basılana dek gelmez)'
              : "Bloğu kaldır (yeri boş kalır; bilgisayar kaydı silinmez)"
          }
          onClick={() => run(() => actions.remove(b))}
        >
          <Trash2 />
        </Button>
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
          {time(sel.a)} – {time(Math.min(sel.b, Date.now()))} · boş alana yeni blok
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
