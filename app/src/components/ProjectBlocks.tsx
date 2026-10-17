import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { api, type Edited, formatDuration, NO_PROJECT, type Report, type Tag, type WorkBlock } from "../api";
import { projectShare, type ProjectShare } from "../lib/blockWindows";
import {
  addDays,
  addMonths,
  daysInMonth,
  formatMonth,
  formatTime,
  formatWeek,
  isoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "../lib/dates";
import { friendlyError, notifyChanged, toast, undoable, useChanged } from "../lib/feedback";
import { useTauriEvent } from "../lib/useTauriEvent";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { useLatest } from "../lib/latest";
import { AppIconStack } from "./AppIcon";
import { BlockDetails } from "./calendar/BlockDetails";
import { blockLabel } from "./calendar/Block";
import { EditContext, useEdit } from "./SessionEdit";
import { ProjectSelect } from "./ProjectSelect";
import { ErrorText } from "./settings";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

type Span = "week" | "month";

/** Listede bir satır: bloğun kendisi ya da (başka projenin bloğundaysa) bu projenin payı. */
type Item = { b: WorkBlock; seconds: number; share?: ProjectShare };

/** Başka projenin bloğunda bundan kısa pay listelenmez. */
const PART_MIN_S = 60;

const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long" });

/**
 * Projeye (ya da müşterinin projelerine) yazılan takvim blokları, gün gün. Bloğa tıklayınca
 * takvimdeki ayrıntı kartı açılır: saatler, proje, kategori, pencereler ve silme buradan da
 * düzenlenir; zaman çizelgesi takvimdeki gibi bloğu izler.
 */
export function ProjectBlocks({ projectIds, showProject = false }: { projectIds: string[]; showProject?: boolean }) {
  const [span, setSpan] = useState<Span>("week");
  const [anchor, setAnchor] = useState(() => today());
  const start = span === "week" ? startOfWeek(anchor) : startOfMonth(anchor);
  const days = span === "week" ? 7 : daysInMonth(start);
  const startIso = isoDate(start);
  const [report, setReport] = useState<{ start: string; days: number; data: Report } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const begin = useLatest();

  const load = useCallback(() => {
    const current = begin();
    api.report(startIso, days, true).then(
      (data) => {
        if (!current()) return;
        setReport({ start: startIso, days, data });
        setError(null);
      },
      (e) => current() && setError(friendlyError(e)),
    );
  }, [startIso, days]);
  useEffect(load, [load]);
  useChanged(load);
  useTauriEvent(api.onSync, load);

  const data = report && report.start === startIso && report.days === days ? report.data : null;
  const tags = useMemo(() => new Map((data?.tags ?? []).map((t) => [t.id, t])), [data]);
  const editCtx = useMemo(
    () => ({
      categories: (data?.tags ?? []).filter((t) => t.kind === "category"),
      projects: (data?.tags ?? []).filter((t) => t.kind === "project"),
      onChanged: load,
    }),
    [data, load],
  );
  const idsKey = projectIds.join(",");
  const groups = useMemo(() => {
    if (!data) return [];
    const ids = new Set(idsKey.split(","));
    const byDay = new Map<string, Item[]>();
    for (const b of data.work.blocks) {
      let item: Item;
      if (b.projectId && ids.has(b.projectId)) item = { b, seconds: b.activeSeconds };
      else {
        // Başka projenin (ya da projesiz) bloğunun içinde bu projeye yazılmış süre: blok listede
        // görünmese de proje toplamına sayılır, buradan bloğun projesine verilebilsin.
        const share = projectShare(data.windows, b.start, b.end, ids);
        if (share.seconds < PART_MIN_S) continue;
        item = { b, seconds: share.seconds, share };
      }
      const day = isoDate(new Date(b.start));
      byDay.set(day, [...(byDay.get(day) ?? []), item]);
    }
    // En yeni gün üstte; gün içinde saat sırasıyla.
    return [...byDay.entries()]
      .sort(([a], [b]) => b.localeCompare(a))
      .map(([day, items]) => ({
        day,
        items: items.sort((x, y) => x.b.start.localeCompare(y.b.start)),
        seconds: items.reduce((n, i) => n + i.seconds, 0),
      }));
  }, [data, idsKey]);
  const total = groups.reduce((n, g) => n + g.seconds, 0);
  const count = groups.reduce((n, g) => n + g.items.length, 0);

  const shift = (n: number) => setAnchor(span === "week" ? addDays(start, n * 7) : addMonths(start, n));
  const isCurrent = isoDate(span === "week" ? startOfWeek(today()) : startOfMonth(today())) === startIso;

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Tabs value={span} onValueChange={(v) => setSpan(v as Span)}>
          <TabsList aria-label="Dönem">
            <TabsTrigger value="week" className="px-3">
              Hafta
            </TabsTrigger>
            <TabsTrigger value="month" className="px-3">
              Ay
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="flex items-center">
          <Button size="icon-sm" variant="ghost" onClick={() => shift(-1)} aria-label="Önceki">
            <ChevronLeft />
          </Button>
          <Button size="icon-sm" variant="ghost" onClick={() => shift(1)} aria-label="Sonraki" disabled={isCurrent}>
            <ChevronRight />
          </Button>
        </div>
        <span className="text-sm font-medium tabular">{span === "week" ? formatWeek(start) : formatMonth(start)}</span>
        {!isCurrent && (
          <Button size="sm" variant="ghost" className="text-muted-foreground" onClick={() => setAnchor(today())}>
            Bugün
          </Button>
        )}
        <div className="flex-1" />
        {data && count > 0 && (
          <span className="text-xs text-muted-foreground tabular">
            {count} blok · {formatDuration(total)}
          </span>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
      {!data ? (
        !error && <div className="skeleton h-48 rounded-xl" />
      ) : groups.length === 0 ? (
        <p className="rounded-xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
          Bu dönemde {showProject ? "bu müşterinin projelerine" : "bu projeye"} yazılmış blok yok.
        </p>
      ) : (
        <EditContext.Provider value={editCtx}>
          <div className="space-y-3">
            {groups.map((g) => (
              <section key={g.day} className="overflow-hidden rounded-xl border bg-card shadow-xs">
                <header className="flex items-center justify-between border-b bg-muted/40 px-3 py-1.5 text-xs">
                  <span className="font-medium">{capitalize(dayFmt.format(new Date(g.items[0].b.start)))}</span>
                  <span className="text-muted-foreground tabular">{formatDuration(g.seconds)}</span>
                </header>
                <ul className="divide-y">
                  {g.items.map(({ b, seconds, share }) => (
                    <BlockRow
                      key={b.start}
                      b={b}
                      seconds={seconds}
                      share={share}
                      ids={projectIds}
                      tags={tags}
                      windows={data.windows}
                      showProject={showProject}
                    />
                  ))}
                </ul>
              </section>
            ))}
          </div>
        </EditContext.Provider>
      )}
      <p className="text-[11px] text-muted-foreground">
        Bu projeye yazılan takvim blokları; başka projenin bloğunun içinde kalan süre “… bloğunda” diye ayrıca görünür.
        Bloğa tıkla: saatleri, projeyi, kategoriyi değiştir ya da sil; zaman çizelgesi de buna göre güncellenir.
      </p>
    </div>
  );
}

function BlockRow({
  b,
  seconds,
  share,
  ids,
  tags,
  windows,
  showProject,
}: {
  b: WorkBlock;
  /** Listede sayılan süre: bloğun tamamı ya da bloktaki bu projenin payı. */
  seconds: number;
  share?: ProjectShare;
  ids: string[];
  tags: Map<string, Tag>;
  windows: Report["windows"];
  showProject: boolean;
}) {
  const project = b.projectId ? tags.get(b.projectId) : undefined;
  const category = b.categoryId ? tags.get(b.categoryId) : undefined;
  // Paydaki projeler (müşteride birden çok olabilir): adları satırda yazılır.
  const shareNames = share ? partNames(windows, share, ids, tags) : "";
  return (
    <li>
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            className="grid w-full grid-cols-[92px_minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-left text-xs transition-colors hover:bg-accent/60 data-[state=open]:bg-accent"
          >
            <span className="tabular">
              {share
                ? `${formatTime(new Date(share.ranges[0][0]))} – ${formatTime(new Date(share.ranges[share.ranges.length - 1][1]))}`
                : `${formatTime(new Date(b.start))} – ${formatTime(new Date(b.end))}`}
            </span>
            {share ? (
              <span className="flex min-w-0 items-center gap-2">
                <i className="h-4 w-[3px] shrink-0 rounded-full border border-dashed border-muted-foreground/60" />
                {showProject && <span className="shrink-0 font-medium">{shareNames}</span>}
                <span className="truncate text-muted-foreground">
                  {project?.name ?? "Projesiz"} bloğunda ({formatTime(new Date(b.start))}–{formatTime(new Date(b.end))})
                </span>
              </span>
            ) : (
              <span className="flex min-w-0 items-center gap-2">
                <i
                  className="h-4 w-[3px] shrink-0 rounded-full"
                  style={{ background: tagColor(showProject ? project : category) }}
                />
                {showProject && project && <span className="shrink-0 font-medium">{project.name}</span>}
                <span className="truncate text-muted-foreground">
                  {category?.name ?? UNCATEGORIZED} · {b.topApps.map((a) => a.appName).join(", ")}
                </span>
                <AppIconStack apps={b.topApps} size={14} className="ml-auto shrink-0" />
              </span>
            )}
            <span className="font-medium tabular">{formatDuration(seconds)}</span>
          </button>
        </PopoverTrigger>
        <PopoverContent side="bottom" align="end" className="w-96 space-y-3">
          {share ? <SharePart b={b} share={share} names={shareNames} tags={tags} /> : <BlockTimes b={b} tags={tags} />}
          <BlockDetails block={b} tags={tags} windows={windows} />
        </PopoverContent>
      </Popover>
    </li>
  );
}

/** Paydaki projelerin adları ("Togg", müşteride "Togg, Portal"). */
function partNames(windows: Report["windows"], share: ProjectShare, ids: string[], tags: Map<string, Tag>) {
  const found = new Set<string>();
  for (const w of windows) {
    if (!w.projectId || !ids.includes(w.projectId)) continue;
    const a = Date.parse(w.start);
    const b = Date.parse(w.end);
    if (share.ranges.some(([x, y]) => a < y && b > x)) found.add(w.projectId);
  }
  return [...found].map((id) => tags.get(id)?.name ?? "Proje").join(", ");
}

/**
 * Başka projenin bloğunun içinde kalan pay: bloğun projesine ya da başka projeye verilir,
 * projesiz yapılır ya da silinir. Yalnızca payın aralıklarına dokunulur; bloğun geri kalanı değişmez.
 */
function SharePart({
  b,
  share,
  names,
  tags,
}: {
  b: WorkBlock;
  share: ProjectShare;
  names: string;
  tags: Map<string, Tag>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const owner = b.projectId ? tags.get(b.projectId) : undefined;
  const projects = useEdit()?.projects ?? [];
  const iso = (t: number) => new Date(t).toISOString();
  const span = share.ranges.map(([a, z]) => `${formatTime(new Date(a))}–${formatTime(new Date(z))}`).join(", ");

  async function apply(edit: (a: string, z: string) => Promise<Edited>, message: string) {
    setBusy(true);
    setError(null);
    try {
      for (const [a, z] of share.ranges) await undoable(edit(iso(a), iso(z)), message);
      notifyChanged();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-2 rounded-md bg-muted/60 px-2.5 py-2 text-xs">
      <p>
        {owner?.name ?? "Projesiz"} bloğunun içinde <strong>{names}</strong> projesine yazılı süre:{" "}
        <strong className="tabular">{formatDuration(share.seconds)}</strong>{" "}
        <span className="text-muted-foreground tabular">({span})</span>
      </p>
      <div className="flex flex-wrap gap-1.5">
        {owner && (
          <Button
            size="sm"
            disabled={busy}
            onClick={() => apply((a, z) => api.setRangeProject(a, z, owner.id), `${owner.name} projesine atandı`)}
          >
            {owner.name} projesine ver
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={() => apply((a, z) => api.setRangeProject(a, z, NO_PROJECT), "Projesiz olarak işaretlendi")}
        >
          Projesiz yap
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="text-muted-foreground hover:text-destructive"
          disabled={busy}
          onClick={() => apply((a, z) => api.deleteRange(a, z), "Süre silindi")}
        >
          Sil
        </Button>
      </div>
      <ProjectSelect
        value=""
        projects={projects}
        placeholder="Başka projeye ata…"
        className="w-full"
        onChange={(id) =>
          id && void apply((a, z) => api.setRangeProject(a, z, id), `${tags.get(id)?.name ?? "Proje"} projesine atandı`)
        }
      />
      {error && <p className="text-[11px] text-destructive selectable">{error}</p>}
    </div>
  );
}

const hhmm = (t: number) => {
  const d = new Date(t);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
};

/** Bloğun saatleri: takvimde kenarı sürüklemekle aynı (`resizeBlock`); kısalan kısım ayrı blok olur. */
function BlockTimes({ b, tags }: { b: WorkBlock; tags: Map<string, Tag> }) {
  const start = +new Date(b.start);
  const end = +new Date(b.end);
  const [from, setFrom] = useState(hhmm(start));
  const [to, setTo] = useState(hhmm(end));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setFrom(hhmm(start));
    setTo(hhmm(end));
  }, [start, end]);

  /** Bloğun gününde "SS:DD"; bitiş başlangıçtan önceyse ertesi gün. */
  const at = (v: string, base: number) => {
    const [h, m] = v.split(":").map(Number);
    const d = new Date(base);
    return +new Date(d.getFullYear(), d.getMonth(), d.getDate(), h, m);
  };
  const newStart = from ? at(from, start) : NaN;
  let newEnd = to ? at(to, start) : NaN;
  if (newEnd <= newStart) newEnd = +addDays(new Date(newEnd), 1);
  const valid = Number.isFinite(newStart) && Number.isFinite(newEnd) && newEnd > newStart;
  const changed = valid && (hhmm(start) !== from || hhmm(end) !== to);

  async function save() {
    if (!changed) return;
    if (newEnd > Date.now()) {
      setError("Bitiş şu andan sonra olamaz.");
      return;
    }
    const iso = (t: number) => new Date(t).toISOString();
    setBusy(true);
    setError(null);
    try {
      await undoable(
        api.resizeBlock(b.start, b.end, iso(newStart), iso(newEnd), blockLabel(b, tags), b.categoryId, b.projectId),
        newStart > start || newEnd < end
          ? `Blok ikiye bölündü: ${formatTime(new Date(newStart))}–${formatTime(new Date(newEnd))}`
          : `Blok ${formatTime(new Date(newStart))}–${formatTime(new Date(newEnd))} oldu`,
      );
      // Açık sayfalar (bu liste dahil) yenilensin.
      notifyChanged();
    } catch (e) {
      setError(friendlyError(e));
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-1">
      <span className="block text-[11px] text-muted-foreground">Saatler</span>
      <form
        className="flex items-center gap-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Input
          type="time"
          className="h-8 w-[92px] text-sm tabular"
          value={from}
          onChange={(e) => setFrom(e.target.value)}
          aria-label="Başlangıç"
        />
        <span className="text-muted-foreground">–</span>
        <Input
          type="time"
          className="h-8 w-[92px] text-sm tabular"
          value={to}
          onChange={(e) => setTo(e.target.value)}
          aria-label="Bitiş"
        />
        <Button type="submit" size="sm" className={cn("ml-auto", !changed && "invisible")} disabled={!changed || busy}>
          Kaydet
        </Button>
      </form>
      {error && <p className="text-[11px] text-destructive selectable">{error}</p>}
    </div>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
