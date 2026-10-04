import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Check,
  ChevronDown,
  ChevronRight,
  FolderKanban,
  Plus,
  Search,
  Tags,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  formatDuration,
  type Budgets,
  type BudgetUsage,
  type Client,
  type Rule,
  type RuleField,
  type Suggestions,
  type Tag,
  type TagKind,
  type UsageTotal,
} from "../api";
import { SuggestionsCard } from "../components/SuggestionsCard";
import { RulePreview } from "../components/RulePreview";
import { BudgetField, BudgetMeter } from "../components/Budget";
import { friendlyError, undoable, useChanged } from "../lib/feedback";
import { ErrorText, Page } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { addDays, isoDate, today } from "../lib/dates";
import { archivedProjects, clientColor, NO_CLIENT, nextColor, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

/** Sayfaya göre metinler: projeler başlıktaki sözcüklerle, kategoriler uygulamalarla çalışır. */
const TEXT = {
  project: {
    title: "Projeler",
    intro:
      "Proje, uygulamalar arası bir iştir: pencere başlığında projenin sözcüğü geçen her şey (Figma dosyası, tarayıcı sekmesi, kod klasörü) o projeye yazılır.",
    add: "Yeni proje adı",
    addHint: "Proje adı, başlıkta aranan ilk sözcük olur; sonra başka sözcükler de ekleyebilirsin.",
    primary: "title" as RuleField,
    empty:
      "Henüz proje yok. Üstten bir ad yaz (örn. müşteri ya da iş adı); başlığında geçen pencereler o projeye sayılır.",
    deleteHint: "Kuralları da silinir. Geçmiş kayıtlar silinmez, projesiz görünür.",
  },
  category: {
    title: "Kategoriler",
    intro:
      "Kategori, zamanın ne tür işe gittiğini gösterir (Tasarım, İletişim…). Her oturum tek kategoriye girer; web sitesi ve başlık kuralları uygulama kurallarından önce gelir.",
    add: "Yeni kategori adı",
    addHint: "Ekledikten sonra kategoriye uygulamaları ata.",
    primary: "app" as RuleField,
    empty: "Henüz kategori yok. Üstten bir ad yaz, sonra uygulamaları ata.",
    deleteHint: "Kuralları da silinir. Geçmiş kayıtlar silinmez, kategorisiz görünür.",
  },
} satisfies Record<TagKind, unknown>;

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

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

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

/** Projenin müşterisi: yerel açılır liste (kartın içinde ayrı katman açmaz). */
function ClientSelect({
  clients,
  value,
  onChange,
  className,
}: {
  clients: Client[];
  value: string | null;
  onChange: (id: string | null) => void;
  className?: string;
}) {
  return (
    <span className={cn("relative inline-flex items-center", className)}>
      <i
        className="pointer-events-none absolute left-2.5 size-2 rounded-full"
        style={{ background: clientColor(clients, value) }}
        aria-hidden
      />
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
        className="h-8 w-full appearance-none rounded-md border bg-transparent pr-7 pl-6 text-xs hover:bg-accent dark:bg-input/30"
        aria-label="Müşteri"
      >
        <option value="">{NO_CLIENT}</option>
        {clients.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
    </span>
  );
}

/** En üstteki ekleme alanı. */
function AddTag({
  kind,
  allTags,
  clients,
  onAdded,
  onError,
}: {
  kind: TagKind;
  allTags: Tag[];
  clients: Client[];
  onAdded: (id: string) => void;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  // Yeni projenin müşterisi; art arda aynı müşteriye proje eklenirken seçili kalır.
  const [client, setClient] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const text = TEXT[kind];
  const same = allTags.find(
    (t) => t.kind === kind && t.name.toLocaleLowerCase("tr") === name.trim().toLocaleLowerCase("tr"),
  );
  const exists = !!same;
  return (
    <form
      className="space-y-1.5 rounded-xl border bg-card px-4 py-3 shadow-xs"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim() || exists) return;
        setBusy(true);
        try {
          const tag = await api.saveTag({ kind, name: name.trim(), color: nextColor(allTags) });
          // Kuralsız proje hiç süre toplamaz: adı, başlıkta aranan sözcük olarak eklenir. Kural
          // eklenemezse proje de kaldırılır; yoksa ad "zaten var" olur ve yeniden denenemez.
          if (kind === "project")
            await api.addRule(tag.id, "title", tag.name).catch(async (err) => {
              await api.deleteTag(tag.id).catch(() => {});
              throw err;
            });
          if (kind === "project" && client) await api.setProjectClient(tag.id, client);
          setName("");
          onAdded(tag.id);
        } catch (err) {
          onError(friendlyError(err));
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="flex items-center gap-2">
        <Input
          className="h-8 flex-1 text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={text.add}
          aria-label={text.add}
        />
        {kind === "project" && clients.length > 0 && (
          <ClientSelect clients={clients} value={client} onChange={setClient} className="w-40" />
        )}
        <Button type="submit" size="sm" disabled={!name.trim() || exists || busy}>
          <Plus /> Ekle
        </Button>
      </div>
      <p className={cn("text-[11px]", exists ? "text-destructive" : "text-muted-foreground")}>
        {exists
          ? `“${name.trim()}” zaten var${same?.archived ? " (arşivde; aşağıdaki Arşiv'den geri alabilirsin)" : ""}.`
          : text.addHint}
      </p>
    </form>
  );
}

/**
 * Başka işletim sisteminin uygulama kuralı mı? Hazır kategoriler hem macOS
 * bundle kimliklerini hem Windows exe adlarını içerir (senkronizasyonla iki
 * sistemde de geçerli); listede yalnızca bu sistemi ilgilendirenler gösterilir.
 */
function foreignRule(r: Rule): boolean {
  if (r.field !== "app") return false;
  const exe = /\.exe$/i.test(r.pattern);
  const platform = document.documentElement.dataset.platform;
  if (platform === "macos") return exe;
  if (platform === "windows") return !exe && r.pattern.includes(".");
  return false;
}

/** Kural özeti: "2 sözcük · 1 site · 1 uygulama". */
function summary(rules: Rule[]) {
  const words = rules.filter((r) => r.field === "title").length;
  const sites = rules.filter((r) => r.field === "domain").length;
  const apps = rules.filter((r) => r.field === "app" && !foreignRule(r)).length;
  const parts = [words && `${words} sözcük`, sites && `${sites} site`, apps && `${apps} uygulama`].filter(Boolean);
  return parts.length ? parts.join(" · ") : "Kural yok: süre toplamaz";
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
  const text = TEXT[tag.kind];
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
            {summary(rules)}
          </span>
        </span>
        {budget && <BudgetMeter usage={budget} dayHours={dayHours} color={tagColor(tag)} compact />}
        <span className="shrink-0 text-xs text-muted-foreground tabular">
          {seconds ? formatDuration(seconds) : "—"}
        </span>
      </button>
      {open && (
        <div className="space-y-4 border-t bg-muted/20 px-4 py-3.5 pl-11">
          <NameAndColor tag={tag} run={run} />
          {tag.kind === "project" && (
            <div className="space-y-1.5">
              <div className="text-[11px] font-medium text-muted-foreground">Müşteri</div>
              {clients.length > 0 ? (
                <ClientSelect
                  clients={clients}
                  value={clientId}
                  onChange={(id) => run(() => api.setProjectClient(tag.id, id))()}
                  className="w-56"
                />
              ) : (
                <p className="text-xs text-muted-foreground">Müşteri yok; kenar çubuğundaki Müşteriler'den ekle.</p>
              )}
            </div>
          )}
          {tag.kind === "project" && (
            <div className="space-y-1.5">
              <div>
                <div className="text-[11px] font-medium text-muted-foreground">Sözleşme bütçesi</div>
                <div className="text-[11px] text-muted-foreground/80">
                  Anlaşılan adam-gün; projeye bugüne kadar yazılan süreyle kıyaslanır, %80'de ve dolunca bildirilir. Bir
                  adam-gün {String(dayHours).replace(".", ",")} saat (Zaman çizelgesi ayarı).
                </div>
              </div>
              <BudgetField value={tag.budgetDays} onSave={(d) => run(() => api.setProjectBudget(tag.id, d))()} />
              {budget && <BudgetMeter usage={budget} dayHours={dayHours} color={tagColor(tag)} className="max-w-xs" />}
            </div>
          )}
          {(text.primary === "title"
            ? (["title", "domain", "app"] as const)
            : (["app", "domain", "title"] as const)
          ).map((field) => (
            <RuleList key={field} tag={tag} field={field} rules={rules} apps={apps} run={run} allTags={allTags} />
          ))}
          <div className="flex justify-end gap-1">
            {tag.kind === "project" && (
              <Button
                variant="ghost"
                size="sm"
                className="text-muted-foreground"
                title="Seçicilerden kalkar ve yeni süre toplamaz; geçmiş kayıtları ve toplamları korunur."
                onClick={run(() => undoable(api.archiveProject(tag.id), `“${tag.name}” arşivlendi`))}
              >
                <Archive /> Arşivle
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground hover:text-destructive"
              title={text.deleteHint}
              onClick={run(() => undoable(api.deleteTag(tag.id), `“${tag.name}” silindi`))}
            >
              <Trash2 /> {tag.kind === "project" ? "Projeyi sil" : "Kategoriyi sil"}
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

function NameAndColor({ tag, run }: { tag: Tag; run: Run }) {
  const [name, setName] = useState(tag.name);
  // Başka cihazdan gelen yeniden adlandırma eski adla ezilmesin.
  useEffect(() => setName(tag.name), [tag.name]);
  return (
    <div className="space-y-1.5">
      <div className="text-[11px] font-medium text-muted-foreground">Ad ve renk</div>
      <div className="flex items-center gap-2">
        <Input
          className="h-8 max-w-xs text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== tag.name && run(() => api.saveTag({ ...tag, name: name.trim() }))()}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Ad"
        />
        <div className="flex gap-1" role="radiogroup" aria-label="Renk">
          {[1, 2, 3, 4, 5, 6, 7, 8].map((c) => (
            <button
              key={c}
              type="button"
              role="radio"
              aria-checked={c === tag.color}
              aria-label={`Renk ${c}`}
              className="grid size-6 place-items-center rounded-full text-white transition-transform hover:scale-110"
              style={{ background: `var(--c${c})` }}
              onClick={run(() => api.saveTag({ ...tag, color: c }))}
            >
              {c === tag.color && <Check className="size-3.5" />}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

const FIELD_TEXT: Record<RuleField, { title: string; hint: string; none: string }> = {
  title: {
    title: "Pencere başlığında geçen sözcükler",
    hint: "Büyük/küçük harf fark etmez; sözcüğün başlıkta bir yerde geçmesi yeter.",
    none: "Sözcük yok",
  },
  app: {
    title: "Uygulamalar",
    hint: "Bu uygulamalarda geçen süre buraya yazılır (başlık kuralı başka yere götürmedikçe).",
    none: "Uygulama yok",
  },
  domain: {
    title: "Web siteleri",
    hint: "Tarayıcıda bu adreslerde geçen süre. “togg.com” alt alan adlarını da kapsar, “github.com/firma” yalnızca o yolun altını.",
    none: "Site yok",
  },
};

/** Sözcük ve site kuralları yazılarak eklenir; uygulamalar listeden seçilir. */
const TYPED: Partial<Record<RuleField, { placeholder: string; label: string }>> = {
  title: { placeholder: "Sözcük ekle…", label: "Başlıkta aranacak sözcük" },
  domain: { placeholder: "örn. jira.togg.com", label: "Web sitesi adresi" },
};

/** Bir türdeki kurallar (sözcükler ya da uygulamalar) ve ekleme. */
function RuleList({
  tag,
  field,
  rules,
  apps,
  run,
  allTags,
}: {
  tag: Tag;
  field: RuleField;
  rules: Rule[];
  apps: UsageTotal[];
  run: Run;
  allTags: Tag[];
}) {
  const [word, setWord] = useState("");
  const appNames = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);
  const mine = rules.filter((r) => r.field === field);
  const shown = mine.filter((r) => !foreignRule(r));
  const hidden = mine.filter(foreignRule);
  const t = FIELD_TEXT[field];
  const typed = TYPED[field];
  return (
    <div className="space-y-1.5">
      <div>
        <div className="text-[11px] font-medium text-muted-foreground">{t.title}</div>
        <div className="text-[11px] text-muted-foreground/80">{t.hint}</div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {shown.map((r) => (
          <span
            key={r.id}
            className="inline-flex h-7 items-center gap-1 rounded-md border bg-card pr-0.5 pl-2 text-xs"
            title={r.pattern}
          >
            <span className="max-w-56 truncate">
              {field === "app" ? (appNames.get(r.pattern) ?? r.pattern) : r.pattern}
            </span>
            <button
              type="button"
              className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
              onClick={run(() =>
                undoable(
                  api.deleteRule(r.id),
                  `“${field === "app" ? (appNames.get(r.pattern) ?? r.pattern) : r.pattern}” kuralı kaldırıldı`,
                ),
              )}
              aria-label={`${r.pattern} kuralını sil`}
            >
              <X className="size-3" />
            </button>
          </span>
        ))}
        {shown.length === 0 && <span className="text-xs text-muted-foreground">{t.none}</span>}
        {hidden.length > 0 && (
          <span className="text-[11px] text-muted-foreground" title={hidden.map((r) => r.pattern).join("\n")}>
            +{hidden.length} başka işletim sistemi için
          </span>
        )}
        {typed ? (
          <form
            className="flex items-center gap-1"
            onSubmit={(e) => {
              e.preventDefault();
              if (!word.trim()) return;
              run(async () => {
                await api.addRule(tag.id, field, word.trim());
                setWord("");
              })();
            }}
          >
            <Input
              className="h-7 w-40 text-xs"
              value={word}
              onChange={(e) => setWord(e.target.value)}
              placeholder={typed.placeholder}
              aria-label={typed.label}
            />
            {word.trim() && (
              <Button type="submit" size="sm" variant="outline" className="h-7">
                Ekle
              </Button>
            )}
          </form>
        ) : (
          <AppPicker
            apps={apps.filter((a) => !mine.some((r) => r.pattern === a.key))}
            onPick={(key) => run(() => api.addRule(tag.id, "app", key))()}
          />
        )}
      </div>
      {typed && word.trim() && <RulePreview tagId={tag.id} field={field} pattern={word} tags={allTags} />}
    </div>
  );
}

/** Aranabilir uygulama listesi (en çok kullanılandan). */
function AppPicker({ apps, onPick }: { apps: UsageTotal[]; onPick: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const list = apps.filter((a) => a.label.toLocaleLowerCase("tr").includes(q.trim().toLocaleLowerCase("tr")));
  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (!o) setQ("");
      }}
    >
      <PopoverTrigger asChild>
        <Button type="button" size="sm" variant="outline" className="h-7">
          <Plus /> Uygulama ekle
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-1.5">
        <Input
          autoFocus
          className="mb-1 h-7 text-xs"
          placeholder="Uygulama ara"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          aria-label="Uygulama ara"
        />
        <ul className="max-h-64 overflow-y-auto">
          {list.length === 0 && <li className="px-2 py-1.5 text-xs text-muted-foreground">Uygulama bulunamadı</li>}
          {list.map((a) => (
            <li key={a.key}>
              <button
                type="button"
                className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-accent"
                onClick={() => {
                  setOpen(false);
                  setQ("");
                  onPick(a.key);
                }}
              >
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
                <span className="shrink-0 text-[11px] text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}
