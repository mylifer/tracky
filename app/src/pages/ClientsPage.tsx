import { useEffect, useMemo, useState } from "react";
import { Building2, ChevronDown, LayoutGrid, Plus, Settings2, Table2, Trash2, X } from "lucide-react";
import { api, formatDuration, type Budgets, type BudgetUsage, type Client, type Tag } from "../api";
import { BudgetField, BudgetMeter } from "../components/Budget";
import {
  ChartCard,
  Delta,
  Legend,
  PeriodTabs,
  RankList,
  ShareBar,
  Sparkline,
  StackedBars,
  Stat,
  Versus,
  weekLabels,
} from "../components/charts";
import { ErrorText } from "../components/settings";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { budgetRatio, budgetState } from "../lib/budget";
import { friendlyError } from "../lib/feedback";
import {
  change,
  formatHours,
  PERIOD_WORD,
  sumOf,
  sumWeekly,
  useInsights,
  WEEKS,
  type Insights,
  type Period,
} from "../lib/insights";
import { activeProjects, clientColor, NO_CLIENT, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

type View = "cards" | "table";
const VIEW_KEY = "kum.clients.view";

/** Bir müşterinin dönem özeti (müşterisiz projeler için `client: null`). */
type Row = {
  key: string;
  client: Client | null;
  name: string;
  color: string;
  projects: Tag[];
  cur: number;
  prev: number;
  weekly: number[];
  /** Bu ay zaman çizelgesine yazılan saat; ayda çizelge yoksa `null`. */
  billed: number | null;
  budget?: BudgetUsage;
};

function rowsOf(data: Insights): { rows: Row[]; none: Row } {
  const make = (client: Client | null, projects: Tag[]): Row => {
    const ids = projects.map((p) => p.id);
    return {
      key: client?.id ?? "none",
      client,
      name: client?.name ?? NO_CLIENT,
      color: clientColor(data.clients, client?.id),
      projects,
      cur: sumOf(data.cur, ids),
      prev: sumOf(data.prev, ids),
      weekly: sumWeekly(data.weekly, ids, data.periods.length || WEEKS),
      billed: data.billed ? sumOf(data.billed, ids) : null,
      budget: client ? data.budgets?.clients.find((b) => b.id === client.id) : undefined,
    };
  };
  return {
    rows: data.clients.map((c) =>
      make(
        c,
        data.projects.filter((p) => data.links[p.id] === c.id),
      ),
    ),
    none: make(
      null,
      data.projects.filter((p) => !data.links[p.id]),
    ),
  };
}

/**
 * Müşteriler: dönem özeti, müşterilere göre haftalık dağılım ve her müşteri için kart (ya da
 * sıralanabilir tablo). Ad, projeler ve bütçe karttan açılan pencerede düzenlenir.
 */
export default function ClientsPage({
  onOpenProjects,
  onOpenProject,
}: {
  onOpenProjects: () => void;
  onOpenProject: (id: string) => void;
}) {
  const [period, setPeriod] = useState<Period>("month");
  const [view, setView] = useState<View>(() => {
    try {
      return localStorage.getItem(VIEW_KEY) === "table" ? "table" : "cards";
    } catch {
      return "cards";
    }
  });
  const { data, error: loadError, reload } = useInsights(period);
  const [editing, setEditing] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(VIEW_KEY, view);
    } catch {
      /* tercih saklanamazsa kartlarla açılır */
    }
  }, [view]);

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      reload();
    } catch (e) {
      setError(friendlyError(e));
    }
  };

  const { rows, none } = useMemo(() => (data ? rowsOf(data) : { rows: [], none: null }), [data]);
  const editClient = data?.clients.find((c) => c.id === editing);
  // Arşivdeki projeler toplamlara sayılır, ama bağlanmayı beklemez.
  const unlinked = data ? activeProjects(data.projects).filter((p) => !data.links[p.id]) : [];

  return (
    <div className="mx-auto w-full max-w-6xl space-y-5 px-6 pt-2 pb-10">
      <h1 className="sr-only">Müşteriler</h1>
      <div className="flex flex-wrap items-center gap-3">
        <PeriodTabs value={period} onChange={setPeriod} />
        <div className="flex-1" />
        <Tabs value={view} onValueChange={(v) => setView(v as View)}>
          <TabsList aria-label="Görünüm">
            <TabsTrigger value="cards" className="px-2.5" title="Kartlar">
              <LayoutGrid /> Kartlar
            </TabsTrigger>
            <TabsTrigger value="table" className="px-2.5" title="Tablo">
              <Table2 /> Tablo
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <Button size="sm" variant={adding ? "secondary" : "default"} onClick={() => setAdding(!adding)}>
          <Plus /> Yeni müşteri
        </Button>
      </div>
      {adding && data && (
        <AddClient
          clients={data.clients}
          onAdded={(id) => {
            setAdding(false);
            reload();
            setEditing(id);
          }}
          onError={setError}
        />
      )}
      <ErrorText>{error ?? loadError}</ErrorText>

      {!data || !none ? (
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="skeleton h-20 rounded-xl" />
            ))}
          </div>
          <div className="skeleton h-60 rounded-xl" />
        </div>
      ) : data.clients.length === 0 ? (
        <div className="flex items-start gap-3 rounded-xl border border-dashed px-4 py-5 text-sm text-muted-foreground">
          <Building2 className="size-5 shrink-0" />
          <p>
            Henüz müşteri yok. “Yeni müşteri” ile bir ad yaz (örn. Togg), sonra projelerini bağla: raporlarda müşteri
            bazında toplam süreyi görürsün.
          </p>
        </div>
      ) : (
        <>
          <Summary rows={rows} data={data} period={period} />
          <Charts rows={rows} none={none} data={data} period={period} />
          {view === "cards" ? (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3">
              {rows.map((r) => (
                <ClientCard
                  key={r.key}
                  row={r}
                  period={period}
                  dayHours={data.budgets?.dayHours ?? 8}
                  onEdit={() => setEditing(r.key)}
                  onOpenProject={onOpenProject}
                />
              ))}
            </div>
          ) : (
            <ClientTable rows={rows} period={period} dayHours={data.budgets?.dayHours ?? 8} onEdit={setEditing} />
          )}
          {unlinked.length > 0 && (
            <p className="px-1 text-xs text-muted-foreground">
              {unlinked.length} proje henüz bir müşteriye bağlı değil:{" "}
              {unlinked
                .slice(0, 4)
                .map((p) => p.name)
                .join(", ")}
              {unlinked.length > 4 ? "…" : ""} ·{" "}
              <button className="underline underline-offset-2 hover:text-foreground" onClick={onOpenProjects}>
                Projeler'de ata
              </button>
            </p>
          )}
        </>
      )}

      <Dialog open={!!editClient} onOpenChange={(o) => !o && setEditing(null)}>
        {editClient && data && (
          <DialogContent>
            <ClientEditor
              client={editClient}
              data={data}
              run={run}
              onDeleted={() => setEditing(null)}
              onOpenProject={(id) => {
                setEditing(null);
                onOpenProject(id);
              }}
            />
          </DialogContent>
        )}
      </Dialog>
    </div>
  );
}

function Summary({ rows, data, period }: { rows: Row[]; data: Insights; period: Period }) {
  const cur = rows.reduce((s, r) => s + r.cur, 0);
  const prev = rows.reduce((s, r) => s + r.prev, 0);
  const billed = data.billed ? rows.reduce((s, r) => s + (r.billed ?? 0), 0) : null;
  const activeClients = rows.filter((r) => r.cur > 0).length;
  const linked = rows.reduce((s, r) => s + activeProjects(r.projects).length, 0);
  const unlinked = activeProjects(data.projects).filter((p) => !data.links[p.id]).length;
  const warned = rows.filter((r) => r.budget && budgetState(r.budget) !== "ok");
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Stat
        label={`Müşteri işleri · ${PERIOD_WORD[period]}`}
        value={formatDuration(cur)}
        hint={<Versus cur={cur} prev={prev} text={data.versus} />}
      />
      <Stat
        label="Bu ay çizelgede"
        value={billed !== null ? formatHours(billed) : "—"}
        hint={
          billed === null
            ? "Bu ay çizelge kaydı yok"
            : period === "month" && cur > 0
              ? `Takip edilene oranı %${Math.round(((billed * 3600) / cur) * 100)}`
              : "Zaman çizelgesine yazılan"
        }
      />
      <Stat
        label="Çalışılan müşteri"
        value={`${activeClients} / ${rows.length}`}
        hint={`${linked} proje${unlinked ? ` · ${unlinked} müşterisiz proje` : ""}`}
      />
      <Stat
        label="Bütçe uyarısı"
        value={warned.length}
        hint={warned.length ? warned.map((r) => r.name).join(", ") : "Bütçesi dolmak üzere olan yok"}
      />
    </div>
  );
}

function Charts({ rows, none, data, period }: { rows: Row[]; none: Row; data: Insights; period: Period }) {
  const { labels, tipLabels } = weekLabels(data.periods);
  // Müşterisiz projelerin süresi de görünsün (gri), ama yalnızca varsa.
  const all = none.weekly.some((v) => v > 0) || none.cur > 0 ? [...rows, none] : rows;
  const series = all.filter((r) => r.weekly.some((v) => v > 0));
  const share = all.filter((r) => r.cur > 0).sort((a, b) => b.cur - a.cur);
  const total = share.reduce((s, r) => s + r.cur, 0);
  return (
    <div className="grid gap-3 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <ChartCard title="Haftalık dağılım" note="Son 26 hafta · saat">
        {series.length ? (
          <>
            <StackedBars
              labels={labels}
              tipLabels={tipLabels}
              series={series.map((r) => ({ key: r.key, name: r.name, color: r.color, values: r.weekly }))}
              partial
            />
            {series.length > 1 && <Legend items={series.map((r) => ({ key: r.key, name: r.name, color: r.color }))} />}
          </>
        ) : (
          <p className="text-xs text-muted-foreground">Son 26 haftada müşteri projelerine düşen süre yok.</p>
        )}
      </ChartCard>
      <ChartCard title={`Pay · ${PERIOD_WORD[period]}`} note={formatDuration(total)}>
        {share.length ? (
          <div className="space-y-4">
            <ShareBar items={share.map((r) => ({ key: r.key, name: r.name, color: r.color, value: r.cur }))} />
            <RankList
              items={share.map((r) => ({
                key: r.key,
                name: r.name,
                color: r.color,
                value: r.cur,
                sub: `%${Math.round((r.cur / (total || 1)) * 100)}`,
              }))}
            />
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">Bu dönemde müşteri projelerine düşen süre yok.</p>
        )}
      </ChartCard>
    </div>
  );
}

function ClientCard({
  row: r,
  period,
  dayHours,
  onEdit,
  onOpenProject,
}: {
  row: Row;
  period: Period;
  dayHours: number;
  onEdit: () => void;
  onOpenProject: (id: string) => void;
}) {
  const projects = activeProjects(r.projects);
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onEdit}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onEdit();
        }
      }}
      className="group flex cursor-default flex-col gap-2.5 rounded-xl border bg-card px-4 py-3.5 shadow-xs transition-colors outline-none hover:border-foreground/15 focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <div className="flex items-center gap-2">
        <i className="size-2.5 shrink-0 rounded-full" style={{ background: r.color }} />
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">{r.name}</span>
        <Settings2 className="size-3.5 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100" aria-hidden />
      </div>
      <div className="flex items-baseline gap-2">
        <span className="text-xl leading-none font-semibold tracking-tight tabular">{formatDuration(r.cur)}</span>
        <span className="text-[11px] text-muted-foreground">{PERIOD_WORD[period]}</span>
        <Delta value={change(r.cur, r.prev)} className="ml-auto text-[11px]" />
      </div>
      <Sparkline values={r.weekly.slice(-12)} color={r.color} height={34} />
      <div className="flex flex-wrap gap-x-2.5 gap-y-1 text-[11px] text-muted-foreground">
        {projects.length === 0 ? (
          <span>Proje yok</span>
        ) : (
          projects.map((p) => (
            <button
              key={p.id}
              type="button"
              className="inline-flex max-w-full items-center gap-1 hover:text-foreground hover:underline"
              onClick={(e) => {
                e.stopPropagation();
                onOpenProject(p.id);
              }}
              title={`${p.name} projesini aç`}
            >
              <i className="size-1.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
              <span className="truncate">{p.name}</span>
            </button>
          ))
        )}
      </div>
      {(r.budget || !!r.billed) && (
        <div className="mt-auto space-y-1.5">
          {!!r.billed && (
            <div className="text-[11px] text-muted-foreground tabular">Bu ay çizelgede {formatHours(r.billed)}</div>
          )}
          {r.budget && <BudgetMeter usage={r.budget} dayHours={dayHours} color={r.color} />}
        </div>
      )}
    </div>
  );
}

type SortKey = "name" | "cur" | "prev" | "change" | "billed" | "budget" | "total";

function ClientTable({
  rows,
  period,
  dayHours,
  onEdit,
}: {
  rows: Row[];
  period: Period;
  dayHours: number;
  onEdit: (id: string) => void;
}) {
  const [sort, setSort] = useState<{ key: SortKey; dir: 1 | -1 }>({ key: "cur", dir: -1 });
  const total = (r: Row) => r.weekly.reduce((a, b) => a + b, 0);
  const value: Record<SortKey, (r: Row) => number | string> = {
    name: (r) => r.name,
    cur: (r) => r.cur,
    prev: (r) => r.prev,
    change: (r) => change(r.cur, r.prev) ?? Infinity,
    billed: (r) => r.billed ?? -1,
    budget: (r) => (r.budget ? budgetRatio(r.budget) : -1),
    total,
  };
  const sorted = [...rows].sort((a, b) => {
    const [x, y] = [value[sort.key](a), value[sort.key](b)];
    const c = typeof x === "string" ? x.localeCompare(y as string, "tr") : x - (y as number);
    return c * sort.dir || a.name.localeCompare(b.name, "tr");
  });
  const head = (key: SortKey, label: string, right = true) => (
    <th
      className={cn("px-3 py-2 font-medium whitespace-nowrap", right ? "text-right" : "text-left")}
      aria-sort={sort.key === key ? (sort.dir > 0 ? "ascending" : "descending") : "none"}
    >
      <button
        type="button"
        className="hover:text-foreground"
        onClick={() => setSort((s) => ({ key, dir: s.key === key ? (s.dir === 1 ? -1 : 1) : key === "name" ? 1 : -1 }))}
      >
        {label}
        {sort.key === key && (sort.dir > 0 ? " ↑" : " ↓")}
      </button>
    </th>
  );
  const sum = (f: (r: Row) => number) => rows.reduce((s, r) => s + f(r), 0);
  const anyBilled = rows.some((r) => r.billed !== null);
  return (
    <div className="overflow-x-auto rounded-xl border bg-card shadow-xs">
      <table className="w-full min-w-max border-collapse text-xs">
        <thead>
          <tr className="border-b text-[11px] text-muted-foreground">
            {head("name", "Müşteri", false)}
            {head("cur", capitalize(PERIOD_WORD[period]))}
            {head("prev", "Önceki dönem")}
            {head("change", "Değişim")}
            <th className="px-3 py-2 text-left font-medium">Son 26 hafta</th>
            {head("billed", "Bu ay çizelgede")}
            {head("budget", "Bütçe", false)}
            {head("total", "26 hafta toplam")}
          </tr>
        </thead>
        <tbody className="divide-y">
          {sorted.map((r) => (
            <tr key={r.key} className="cursor-default hover:bg-muted/40" onClick={() => onEdit(r.key)}>
              <td className="px-3 py-2">
                <span className="flex items-center gap-2">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: r.color }} />
                  <span className="min-w-0">
                    <span className="block truncate font-medium">{r.name}</span>
                    <span className="block text-[11px] text-muted-foreground">
                      {activeProjects(r.projects).length} proje
                    </span>
                  </span>
                </span>
              </td>
              <td className="px-3 py-2 text-right font-semibold tabular">{formatDuration(r.cur)}</td>
              <td className="px-3 py-2 text-right text-muted-foreground tabular">{formatDuration(r.prev)}</td>
              <td className="px-3 py-2 text-right">
                <Delta value={change(r.cur, r.prev)} />
              </td>
              <td className="w-32 px-3 py-2">
                <Sparkline values={r.weekly} color={r.color} height={24} />
              </td>
              <td className="px-3 py-2 text-right tabular">{r.billed !== null ? formatHours(r.billed) : "—"}</td>
              <td className="w-44 px-3 py-2">
                {r.budget ? (
                  <BudgetMeter usage={r.budget} dayHours={dayHours} color={r.color} compact />
                ) : (
                  <span className="text-muted-foreground">—</span>
                )}
              </td>
              <td className="px-3 py-2 text-right tabular">{formatDuration(total(r))}</td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr className="border-t bg-muted/40 font-semibold">
            <td className="px-3 py-2">Toplam</td>
            <td className="px-3 py-2 text-right tabular">{formatDuration(sum((r) => r.cur))}</td>
            <td className="px-3 py-2 text-right tabular">{formatDuration(sum((r) => r.prev))}</td>
            <td className="px-3 py-2 text-right">
              <Delta
                value={change(
                  sum((r) => r.cur),
                  sum((r) => r.prev),
                )}
              />
            </td>
            <td />
            <td className="px-3 py-2 text-right tabular">{anyBilled ? formatHours(sum((r) => r.billed ?? 0)) : "—"}</td>
            <td />
            <td className="px-3 py-2 text-right tabular">{formatDuration(sum(total))}</td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

function AddClient({
  clients,
  onAdded,
  onError,
}: {
  clients: Client[];
  onAdded: (id: string) => void;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const exists = clients.some((c) => c.name.toLocaleLowerCase("tr") === name.trim().toLocaleLowerCase("tr"));
  return (
    <form
      className="space-y-1.5 rounded-xl border bg-card px-4 py-3 shadow-xs"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim() || exists) return;
        setBusy(true);
        try {
          const c = await api.saveClient(null, name.trim());
          setName("");
          onAdded(c.id);
        } catch (err) {
          onError(friendlyError(err));
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="flex items-center gap-2">
        <Input
          autoFocus
          className="h-8 flex-1 text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Yeni müşteri adı"
          aria-label="Yeni müşteri adı"
        />
        <Button type="submit" size="sm" disabled={!name.trim() || exists || busy}>
          <Plus /> Ekle
        </Button>
      </div>
      <p className={cn("text-[11px]", exists ? "text-destructive" : "text-muted-foreground")}>
        {exists ? `“${name.trim()}” zaten var.` : "Ekledikten sonra projelerini bağla. Bir proje tek müşteriye aittir."}
      </p>
    </form>
  );
}

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

/** Müşteri ayarları: ad, bağlı projeler, sözleşme bütçesi ve silme. */
function ClientEditor({
  client,
  data,
  run,
  onDeleted,
  onOpenProject,
}: {
  client: Client;
  data: Insights;
  run: Run;
  onDeleted: () => void;
  onOpenProject: (id: string) => void;
}) {
  const [name, setName] = useState(client.name);
  useEffect(() => setName(client.name), [client.name]);
  const { links, clients } = data;
  const budgets: Budgets | null = data.budgets;
  const color = clientColor(clients, client.id);
  const projects = data.projects.filter((p) => links[p.id] === client.id);
  const others = activeProjects(data.projects).filter((p) => links[p.id] !== client.id);
  const dayHours = budgets?.dayHours ?? 8;
  const budget = budgets?.clients.find((b) => b.id === client.id);
  const projectBudgets = projects.flatMap((p) => {
    const b = budgets?.projects.find((x) => x.id === p.id);
    return b ? [{ project: p, budget: b }] : [];
  });
  const clientName = (id: string | undefined) => clients.find((c) => c.id === id)?.name;
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2 pr-6">
        <i className="size-2.5 shrink-0 rounded-full" style={{ background: color }} />
        <DialogTitle className="truncate">{client.name}</DialogTitle>
      </div>
      <DialogDescription className="-mt-3">Ad, bağlı projeler ve sözleşme bütçesi.</DialogDescription>
      <div className="space-y-1.5">
        <div className="text-[11px] font-medium text-muted-foreground">Ad</div>
        <Input
          className="h-8 max-w-xs text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== client.name && run(() => api.saveClient(client.id, name.trim()))()}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Müşteri adı"
        />
      </div>
      <div className="space-y-1.5">
        <div className="text-[11px] font-medium text-muted-foreground">Projeleri</div>
        <div className="flex flex-wrap items-center gap-1.5">
          {projects.map((p) => (
            <span
              key={p.id}
              className="inline-flex h-7 items-center gap-1.5 rounded-md border bg-card pr-0.5 pl-2 text-xs"
            >
              <i className="size-2 rounded-full" style={{ background: tagColor(p) }} />
              <button
                type="button"
                className={cn("max-w-48 truncate hover:underline", p.archived && "text-muted-foreground")}
                onClick={() => onOpenProject(p.id)}
                title="Projeyi aç"
              >
                {p.name}
                {p.archived ? " (arşiv)" : ""}
              </button>
              <button
                type="button"
                className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
                onClick={run(() => api.setProjectClient(p.id, null))}
                aria-label={`${p.name} projesini bu müşteriden çıkar`}
                title="Müşteriden çıkar (proje silinmez)"
              >
                <X className="size-3" />
              </button>
            </span>
          ))}
          {projects.length === 0 && <span className="text-xs text-muted-foreground">Proje yok</span>}
          {others.length > 0 && (
            <span className="relative inline-flex items-center">
              <select
                value=""
                onChange={(e) => e.target.value && run(() => api.setProjectClient(e.target.value, client.id))()}
                className="h-7 appearance-none rounded-md border bg-transparent pr-7 pl-2 text-xs hover:bg-accent dark:bg-input/30"
                aria-label="Proje bağla"
              >
                <option value="" disabled>
                  + Proje bağla…
                </option>
                {others.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                    {links[p.id] ? ` (şu an: ${clientName(links[p.id])})` : ""}
                  </option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" />
            </span>
          )}
        </div>
      </div>
      <div className="space-y-1.5">
        <div>
          <div className="text-[11px] font-medium text-muted-foreground">Sözleşme bütçesi</div>
          <div className="text-[11px] text-muted-foreground/80">
            Müşteriyle anlaşılan toplam adam-gün; bağlı projelerin bugüne kadarki süresiyle kıyaslanır, %80'de ve
            dolunca bildirilir. Proje bütçeleri projenin ayarlarında.
          </div>
        </div>
        <BudgetField value={client.budgetDays} onSave={(d) => run(() => api.setClientBudget(client.id, d))()} />
        {budget && <BudgetMeter usage={budget} dayHours={dayHours} color={color} className="max-w-xs" />}
      </div>
      {projectBudgets.length > 0 && (
        <div className="space-y-1.5">
          <div className="text-[11px] font-medium text-muted-foreground">Proje bütçeleri</div>
          <ul className="max-w-sm space-y-2">
            {projectBudgets.map(({ project, budget }) => (
              <li key={project.id} className="space-y-1">
                <span className="flex items-center gap-1.5 text-xs">
                  <i className="size-2 rounded-full" style={{ background: tagColor(project) }} />
                  {project.name}
                </span>
                <BudgetMeter usage={budget} dayHours={dayHours} color={tagColor(project)} />
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="flex justify-end">
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="ghost" size="sm" className="text-muted-foreground hover:text-destructive">
              <Trash2 /> Müşteriyi sil
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>“{client.name}” silinsin mi?</AlertDialogTitle>
              <AlertDialogDescription>
                Projeleri ve kayıtları silinmez; projeler müşterisiz kalır.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Vazgeç</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={async () => {
                  await run(() => api.deleteClient(client.id))();
                  onDeleted();
                }}
              >
                Sil
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </div>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
