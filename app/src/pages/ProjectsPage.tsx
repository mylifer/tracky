import { useEffect, useMemo, useState } from "react";
import { ArchiveRestore, ChevronRight, FolderKanban, Plus, Search, Settings2 } from "lucide-react";
import { api, formatDuration, type Suggestions, type Tag, type UsageTotal } from "../api";
import { SuggestionsCard } from "../components/SuggestionsCard";
import { BudgetMeter } from "../components/Budget";
import { Delta, PeriodTabs, Sparkline } from "../components/charts";
import { AddTag, ruleSummary } from "../components/TagEditor";
import { ErrorText } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { budgetRatio } from "../lib/budget";
import { friendlyError, undoable } from "../lib/feedback";
import { change, formatHours, PERIOD_WORD, useInsights, type Insights, type Period } from "../lib/insights";
import { clientColor, NO_CLIENT, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import ProjectDetail, { type DetailTab } from "./projects/ProjectDetail";

type Sort = "time" | "change" | "budget" | "name";
const SORTS: { id: Sort; label: string }[] = [
  { id: "time", label: "Süreye göre" },
  { id: "change", label: "Değişime göre" },
  { id: "budget", label: "Bütçe doluluğuna göre" },
  { id: "name", label: "Ada göre" },
];
/** Müşteri süzgecinde "hepsi" ve "müşterisiz". */
const ALL = "all";
const NONE = "none";
/** Bu kadar projeden sonra arama kutusu görünür. */
const SEARCH_FROM = 9;

/**
 * Projeler: her proje bir kart (dönemdeki süre ve değişim, 12 haftalık eğilim, bütçe). Karta
 * tıklayınca proje detayı (rapor, kurallar ve ayarlar) açılır. `open` dışarıdan açılacak proje
 * (Müşteriler sayfasından gelince).
 */
export default function ProjectsPage({
  open,
  onOpen,
  onSuggestions,
}: {
  open: string | null;
  onOpen: (id: string | null) => void;
  onSuggestions?: (s: Suggestions) => void;
}) {
  const [period, setPeriod] = useState<Period>("week");
  const { data, error: loadError, reload } = useInsights(period);
  const [tab, setTab] = useState<DetailTab>("overview");
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [suggestions, setSuggestions] = useState<Suggestions>({ projects: [], categories: [] });
  const [error, setError] = useState<string | null>(null);

  const loadSuggestions = () =>
    api.suggestions().then(
      (s) => {
        setSuggestions(s);
        onSuggestions?.(s);
      },
      () => {},
    );
  useEffect(() => {
    api.knownApps().then(setApps, () => {});
    loadSuggestions();
  }, []);

  const changed = () => {
    reload();
    loadSuggestions();
  };
  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      changed();
    } catch (e) {
      setError(friendlyError(e));
    }
  };

  const project = open && data ? data.projects.find((p) => p.id === open) : undefined;
  // Silinen proje: listeye dön.
  useEffect(() => {
    if (open && data && !project) onOpen(null);
  }, [open, data, project, onOpen]);

  if (project && data)
    return (
      <ProjectDetail
        project={project}
        data={data}
        period={period}
        onPeriod={setPeriod}
        tab={tab}
        onTab={setTab}
        apps={apps}
        run={run}
        error={error ?? loadError}
        onBack={() => {
          onOpen(null);
          setTab("overview");
        }}
      />
    );

  return (
    <div className="mx-auto w-full max-w-6xl space-y-5 px-6 pt-2 pb-10">
      <h1 className="sr-only">Projeler</h1>
      <ProjectList
        data={data}
        period={period}
        onPeriod={setPeriod}
        suggestions={suggestions}
        onChanged={changed}
        error={error ?? loadError}
        onError={setError}
        run={run}
        onOpen={(id, t) => {
          setTab(t);
          onOpen(id);
        }}
      />
    </div>
  );
}

function ProjectList({
  data,
  period,
  onPeriod,
  suggestions,
  onChanged,
  error,
  onError,
  run,
  onOpen,
}: {
  data: Insights | null;
  period: Period;
  onPeriod: (p: Period) => void;
  suggestions: Suggestions;
  onChanged: () => void;
  error: string | null;
  onError: (e: string) => void;
  run: (f: () => Promise<unknown>) => () => Promise<void>;
  onOpen: (id: string, tab: DetailTab) => void;
}) {
  const [sort, setSort] = useState<Sort>("time");
  const [client, setClient] = useState<string>(ALL);
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState(false);
  const [showArchive, setShowArchive] = useState(false);

  const active = useMemo(() => (data ? data.projects.filter((p) => !p.archived) : []), [data]);
  const archived = data ? data.projects.filter((p) => p.archived) : [];
  const shown = useMemo(() => {
    if (!data) return [];
    const q = query.trim().toLocaleLowerCase("tr");
    const list = active.filter(
      (p) =>
        (client === ALL || (client === NONE ? !data.links[p.id] : data.links[p.id] === client)) &&
        (!q || p.name.toLocaleLowerCase("tr").includes(q)),
    );
    const cur = (p: Tag) => data.cur.get(p.id) ?? 0;
    const ratio = (p: Tag) => {
      const b = data.budgets?.projects.find((x) => x.id === p.id);
      return b ? budgetRatio(b) : -1;
    };
    const delta = (p: Tag) => change(cur(p), data.prev.get(p.id) ?? 0) ?? Infinity;
    const by: Record<Sort, (a: Tag, b: Tag) => number> = {
      time: (a, b) => cur(b) - cur(a),
      change: (a, b) => delta(b) - delta(a),
      budget: (a, b) => ratio(b) - ratio(a),
      name: (a, b) => a.name.localeCompare(b.name, "tr"),
    };
    return [...list].sort((a, b) => by[sort](a, b) || a.name.localeCompare(b.name, "tr"));
  }, [data, active, client, query, sort]);

  const unlinked = data ? active.some((p) => !data.links[p.id]) : false;

  return (
    <>
      <div className="flex flex-wrap items-center gap-3">
        <PeriodTabs value={period} onChange={onPeriod} />
        <Select value={sort} onValueChange={(v) => setSort(v as Sort)}>
          <SelectTrigger size="sm" className="w-48" aria-label="Sıralama">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SORTS.map((s) => (
              <SelectItem key={s.id} value={s.id}>
                {s.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex-1" />
        {active.length >= SEARCH_FROM && (
          <div className="relative w-52">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-8 pl-8 text-xs"
              placeholder="Projelerde ara"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Projelerde ara"
            />
          </div>
        )}
        <Button size="sm" variant={adding ? "secondary" : "default"} onClick={() => setAdding(!adding)}>
          <Plus /> Yeni proje
        </Button>
      </div>
      {adding && data && (
        <AddTag
          kind="project"
          allTags={data.tags}
          clients={data.clients}
          onAdded={(id) => {
            setAdding(false);
            onChanged();
            onOpen(id, "settings");
          }}
          onError={(e) => onError(friendlyError(e))}
        />
      )}
      <ErrorText>{error}</ErrorText>
      {data && (
        <SuggestionsCard
          suggestions={{ projects: suggestions.projects, categories: [] }}
          tags={data.tags}
          onChanged={onChanged}
          onError={onError}
        />
      )}

      {data && data.clients.length > 0 && active.length > 0 && (
        <div className="flex flex-wrap gap-1.5" role="group" aria-label="Müşteri">
          <Chip on={client === ALL} onClick={() => setClient(ALL)}>
            Tümü
          </Chip>
          {data.clients.map((c) => (
            <Chip key={c.id} on={client === c.id} onClick={() => setClient(c.id)}>
              <i className="size-2 rounded-full" style={{ background: clientColor(data.clients, c.id) }} />
              {c.name}
            </Chip>
          ))}
          {unlinked && (
            <Chip on={client === NONE} onClick={() => setClient(NONE)}>
              <i className="size-2 rounded-full" style={{ background: "var(--c0)" }} />
              {NO_CLIENT}
            </Chip>
          )}
        </div>
      )}

      {!data ? (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-3">
          {[0, 1, 2].map((i) => (
            <div key={i} className="skeleton h-44 rounded-xl" />
          ))}
        </div>
      ) : active.length === 0 ? (
        <div className="flex items-start gap-3 rounded-xl border border-dashed px-4 py-5 text-sm text-muted-foreground">
          <FolderKanban className="size-5 shrink-0" />
          <p>
            Henüz proje yok. “Yeni proje” ile bir ad yaz (örn. müşteri ya da iş adı); başlığında geçen pencereler o
            projeye sayılır.
          </p>
        </div>
      ) : shown.length === 0 ? (
        <p className="px-1 text-xs text-muted-foreground">Bu süzgece uyan proje yok.</p>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-3">
          {shown.map((p) => (
            <ProjectCard key={p.id} project={p} data={data} period={period} onOpen={(t) => onOpen(p.id, t)} />
          ))}
        </div>
      )}

      {archived.length > 0 && data && (
        <section className="space-y-2">
          <button
            type="button"
            className="flex items-center gap-1.5 px-1 text-[13px] font-semibold hover:text-foreground"
            onClick={() => setShowArchive(!showArchive)}
            aria-expanded={showArchive}
          >
            <ChevronRight
              className={cn("size-3.5 text-muted-foreground transition-transform", showArchive && "rotate-90")}
            />
            Arşiv <span className="font-normal text-muted-foreground tabular">{archived.length}</span>
          </button>
          {showArchive && (
            <>
              <p className="px-1 text-xs text-muted-foreground">
                Arşivdeki projeler seçicilerde görünmez ve yeni süre toplamaz; geçmiş kayıtları ve rapor toplamları
                korunur.
              </p>
              <ul className="divide-y rounded-xl border bg-card shadow-xs">
                {archived.map((t) => {
                  const budget = data.budgets?.projects.find((b) => b.id === t.id);
                  return (
                    <li key={t.id} className="flex items-center gap-3 px-4 py-2">
                      <i className="size-2.5 shrink-0 rounded-full opacity-60" style={{ background: tagColor(t) }} />
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate text-left text-[13px] text-muted-foreground hover:text-foreground hover:underline"
                        onClick={() => onOpen(t.id, "overview")}
                      >
                        {t.name}
                      </button>
                      {budget && (
                        <BudgetMeter
                          usage={budget}
                          dayHours={data.budgets?.dayHours ?? 8}
                          color={tagColor(t)}
                          compact
                        />
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={run(() => undoable(api.unarchiveProject(t.id), `“${t.name}” arşivden çıkarıldı`))}
                      >
                        <ArchiveRestore /> Arşivden çıkar
                      </Button>
                    </li>
                  );
                })}
              </ul>
            </>
          )}
        </section>
      )}
    </>
  );
}

function Chip({ on, onClick, children }: { on: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-xs transition-colors",
        on ? "border-transparent bg-foreground text-background" : "bg-card hover:bg-accent",
      )}
    >
      {children}
    </button>
  );
}

function ProjectCard({
  project: p,
  data,
  period,
  onOpen,
}: {
  project: Tag;
  data: Insights;
  period: Period;
  onOpen: (tab: DetailTab) => void;
}) {
  const cur = data.cur.get(p.id) ?? 0;
  const prev = data.prev.get(p.id) ?? 0;
  const weekly = data.weekly.get(p.id) ?? [];
  const client = data.clients.find((c) => c.id === data.links[p.id]);
  const rules = data.rules.filter((r) => r.tagId === p.id);
  const budget = data.budgets?.projects.find((b) => b.id === p.id);
  const color = tagColor(p);
  const noRules = rules.length === 0;
  const total = weekly.reduce((a, b) => a + b, 0);
  const billed = data.billed?.get(p.id);
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onOpen("overview")}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen("overview");
        }
      }}
      className="group flex cursor-default flex-col gap-2.5 rounded-xl border bg-card px-4 py-3.5 text-left shadow-xs transition-colors outline-none hover:border-foreground/15 focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <div className="flex items-center gap-2">
        <i className="size-2.5 shrink-0 rounded-full" style={{ background: color }} />
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">{p.name}</span>
        <button
          type="button"
          className="-my-1 -mr-1.5 grid size-6 place-items-center rounded-md text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-accent hover:text-foreground focus-visible:opacity-100"
          onClick={(e) => {
            e.stopPropagation();
            onOpen("settings");
          }}
          aria-label={`${p.name}: kurallar ve ayarlar`}
          title="Kurallar ve ayarlar"
        >
          <Settings2 className="size-3.5" />
        </button>
      </div>
      <div
        className={cn(
          "-mt-2 truncate text-[11px]",
          noRules ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground",
        )}
      >
        {client?.name ?? NO_CLIENT} · {noRules ? ruleSummary(rules) : `${PERIOD_WORD[period]}`}
      </div>
      <div className="flex items-baseline gap-2">
        <span className="text-xl leading-none font-semibold tracking-tight tabular">{formatDuration(cur)}</span>
        <Delta value={change(cur, prev)} className="ml-auto text-[11px]" />
      </div>
      <Sparkline values={weekly.slice(-12)} color={color} height={34} />
      <div className="flex justify-between text-[11px] text-muted-foreground tabular">
        <span title="Son 26 haftanın toplamı">26 hafta {formatDuration(total)}</span>
        {billed !== undefined && <span title="Bu ay zaman çizelgesine yazılan">Çizelgede {formatHours(billed)}</span>}
      </div>
      {budget && <BudgetMeter usage={budget} dayHours={data.budgets?.dayHours ?? 8} color={color} />}
    </div>
  );
}
