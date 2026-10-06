import { useCallback, useEffect, useState } from "react";
import { ArchiveRestore, ChevronRight, FolderKanban, Search, Tags } from "lucide-react";
import {
  api,
  formatDuration,
  type Budgets,
  type BudgetUsage,
  type Client,
  type Rule,
  type Suggestions,
  type Tag,
  type TagKind,
  type UsageTotal,
} from "../api";
import { SuggestionsCard } from "../components/SuggestionsCard";
import { BudgetMeter } from "../components/Budget";
import { AddTag, foreignRule, ruleSummary, TagEditor, TEXT, type Run } from "../components/TagEditor";
import { friendlyError, undoable, useChanged } from "../lib/feedback";
import { ErrorText, Page } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { addDays, isoDate, today } from "../lib/dates";
import { archivedProjects, clientColor, NO_CLIENT, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

/** Bu kadar öğeden sonra arama kutusu görünür. */
const SEARCH_FROM = 7;

/**
 * Projeler ya da kategoriler: üstte ekleme, öneriler; altta her öğe kapalı bir satır (renk,
 * ad, kural özeti, son 7 gün). Satıra tıklayınca kuralları düzenlenir.
 */
export default function TagsPage({
  kind,
  onSuggestions,
}: {
  kind: TagKind;
  /** Bekleyen öneri sayıları (kenar çubuğu rozetleri). */
  onSuggestions?: (s: Suggestions) => void;
}) {
  const text = TEXT[kind];
  const [tags, setTags] = useState<Tag[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [links, setLinks] = useState<Record<string, string>>({});
  const [rules, setRules] = useState<Rule[]>([]);
  const [suggestions, setSuggestions] = useState<Suggestions>({ projects: [], categories: [] });
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [usage, setUsage] = useState<Map<string, number>>(new Map());
  const [budgets, setBudgets] = useState<Budgets | null>(null);
  const [showArchive, setShowArchive] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    let t;
    try {
      t = await api.taxonomy();
    } catch (e) {
      // Boş liste "henüz proje yok" gibi görünmesin.
      setError(friendlyError(e));
      return;
    }
    setTags(t.tags);
    setClients(t.clients);
    setLinks(t.projectClients);
    setRules(t.rules);
    // Kural eklenip silindikçe öneriler değişir (örn. proje eklenince önerisi kalkar).
    const s = await api.suggestions().catch(() => null);
    if (s) {
      setSuggestions(s);
      onSuggestions?.(s);
    }
    if (kind === "project") setBudgets(await api.budgets().catch(() => null));
    const r = await api.report(isoDate(addDays(today(), -6)), 7, false).catch(() => null);
    if (r) {
      const buckets = kind === "project" ? r.projects : r.categories;
      setUsage(new Map(buckets.filter((b) => b.id).map((b) => [b.id!, b.seconds])));
    }
  }, [kind, onSuggestions]);

  useEffect(() => {
    load();
    api.knownApps().then(setApps, () => {});
  }, [load]);
  useChanged(load);

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      await load();
    } catch (e) {
      setError(friendlyError(e));
    }
  };

  // Arşivdeki projeler ayrı, kapalı bir bölümde; listede ve aramada yalnızca etkinler.
  const mine = tags.filter((t) => t.kind === kind && !t.archived);
  const archived = kind === "project" ? archivedProjects(tags) : [];
  const budgetOf = (id: string) => budgets?.projects.find((b) => b.id === id);
  const shown = query.trim()
    ? mine.filter((t) => t.name.toLocaleLowerCase("tr").includes(query.trim().toLocaleLowerCase("tr")))
    : mine;
  const filtered: Suggestions =
    kind === "project"
      ? { projects: suggestions.projects, categories: [] }
      : { projects: [], categories: suggestions.categories };

  return (
    <Page title={text.title}>
      <div className="space-y-2.5">
        <p className="px-1 text-[13px] text-muted-foreground">{text.intro}</p>
        <AddTag
          kind={kind}
          allTags={tags}
          clients={clients}
          onAdded={async (id) => {
            await load();
            setOpen(id);
          }}
          onError={(e) => setError(friendlyError(e))}
        />
      </div>
      <ErrorText>{error}</ErrorText>
      <SuggestionsCard suggestions={filtered} tags={tags} onChanged={load} onError={setError} />

      <section className="space-y-2">
        <div className="flex items-center gap-2 px-1">
          <h2 className="text-[13px] font-semibold">
            {text.title} <span className="font-normal text-muted-foreground tabular">{mine.length}</span>
          </h2>
          <span className="ml-auto text-[11px] text-muted-foreground">Son 7 gün</span>
        </div>
        {mine.length >= SEARCH_FROM && (
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-8 pl-8 text-xs"
              placeholder={`${text.title} içinde ara`}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label={`${text.title} içinde ara`}
            />
          </div>
        )}
        {mine.length === 0 ? (
          <div className="flex items-start gap-3 rounded-xl border border-dashed px-4 py-5 text-sm text-muted-foreground">
            {kind === "project" ? <FolderKanban className="size-5 shrink-0" /> : <Tags className="size-5 shrink-0" />}
            <p>{text.empty}</p>
          </div>
        ) : shown.length === 0 ? (
          <p className="px-1 text-xs text-muted-foreground">“{query}” ile eşleşen yok.</p>
        ) : (
          // Projeler müşteriye göre gruplanır (müşteri yoksa tek liste); müşterisizler en sonda.
          groups(kind, shown, clients, links).map((g) => (
            <div key={g.id ?? "none"} className="space-y-1.5">
              {g.title && (
                <div className="flex items-center gap-2 px-1 pt-1 text-xs font-medium">
                  <i className="size-2 rounded-full" style={{ background: clientColor(clients, g.id) }} />
                  <span className={cn(!g.id && "text-muted-foreground")}>{g.title}</span>
                  <span className="ml-auto font-normal text-muted-foreground tabular">
                    {formatDuration(g.tags.reduce((s, t) => s + (usage.get(t.id) ?? 0), 0))}
                  </span>
                </div>
              )}
              <ul className="divide-y rounded-xl border bg-card shadow-xs">
                {g.tags.map((t) => (
                  <TagItem
                    key={t.id}
                    tag={t}
                    rules={rules.filter((r) => r.tagId === t.id)}
                    apps={apps}
                    clients={clients}
                    clientId={links[t.id] ?? null}
                    seconds={usage.get(t.id) ?? 0}
                    budget={budgetOf(t.id)}
                    dayHours={budgets?.dayHours ?? 8}
                    open={open === t.id}
                    onToggle={() => setOpen(open === t.id ? null : t.id)}
                    run={run}
                    allTags={tags}
                  />
                ))}
              </ul>
            </div>
          ))
        )}
      </section>
      {archived.length > 0 && (
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
                  const budget = budgetOf(t.id);
                  return (
                    <li key={t.id} className="flex items-center gap-3 px-4 py-2">
                      <i className="size-2.5 shrink-0 rounded-full opacity-60" style={{ background: tagColor(t) }} />
                      <span className="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">{t.name}</span>
                      {budget && (
                        <BudgetMeter usage={budget} dayHours={budgets?.dayHours ?? 8} color={tagColor(t)} compact />
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
    </Page>
  );
}

/** Projeleri müşteriye göre gruplar; kategoriler ya da hiç müşteri yoksa başlıksız tek grup. */
function groups(kind: TagKind, tags: Tag[], clients: Client[], links: Record<string, string>) {
  if (kind !== "project" || clients.length === 0) return [{ id: null, title: null, tags }];
  const out = clients
    .map((c) => ({
      id: c.id as string | null,
      title: c.name as string | null,
      tags: tags.filter((t) => links[t.id] === c.id),
    }))
    .filter((g) => g.tags.length > 0);
  const none = tags.filter((t) => !links[t.id]);
  if (none.length) out.push({ id: null, title: NO_CLIENT, tags: none });
  return out;
}

/** Kapalıyken tek satır; açılınca ad, renk, kurallar ve silme. */
function TagItem({
  tag,
  rules,
  apps,
  clients,
  clientId,
  seconds,
  budget,
  dayHours,
  open,
  onToggle,
  run,
  allTags,
}: {
  tag: Tag;
  rules: Rule[];
  apps: UsageTotal[];
  clients: Client[];
  clientId: string | null;
  seconds: number;
  /** Projenin sözleşme bütçesi ve harcanan (tüm zamanlar). */
  budget?: BudgetUsage;
  dayHours: number;
  open: boolean;
  onToggle: () => void;
  run: Run;
  allTags: Tag[];
}) {
  const empty = rules.filter((r) => !foreignRule(r)).length === 0;
  return (
    <li>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-accent/50"
      >
        <ChevronRight
          className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
        />
        <i className="size-2.5 shrink-0 rounded-full" style={{ background: `var(--c${tag.color})` }} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium">{tag.name}</span>
          <span
            className={cn(
              "block truncate text-[11px]",
              empty ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground",
            )}
          >
            {ruleSummary(rules)}
          </span>
        </span>
        {budget && <BudgetMeter usage={budget} dayHours={dayHours} color={tagColor(tag)} compact />}
        <span className="shrink-0 text-xs text-muted-foreground tabular">
          {seconds ? formatDuration(seconds) : "—"}
        </span>
      </button>
      {open && (
        <div className="border-t bg-muted/20 px-4 py-3.5 pl-11">
          <TagEditor
            tag={tag}
            rules={rules}
            apps={apps}
            clients={clients}
            clientId={clientId}
            budget={budget}
            dayHours={dayHours}
            run={run}
            allTags={allTags}
          />
        </div>
      )}
    </li>
  );
}
