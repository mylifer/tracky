import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AppWindow,
  CalendarClock,
  CheckCircle2,
  ChevronDown,
  Coffee,
  EyeOff,
  FileSpreadsheet,
  FolderKanban,
  Globe,
  MoreHorizontal,
  Sparkles,
  Wand2,
} from "lucide-react";
import {
  api,
  formatDuration,
  NO_PROJECT,
  type RuleField,
  type Tag,
  type Unassigned,
  type UnassignedGroup,
  type UnassignedItem,
} from "../api";
import { ProjectSelect } from "../components/ProjectSelect";
import { RulePreview } from "../components/RulePreview";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addDays, formatDate, formatTime, isoDate, parseIsoDate, startOfWeek, today } from "../lib/dates";
import { friendlyError, notifyChanged, toast, undoable, useChanged } from "../lib/feedback";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

type Period = "today" | "week" | "lastWeek" | "days30" | "custom";
const PERIODS: { id: Period; label: string }[] = [
  { id: "today", label: "Bugün" },
  { id: "week", label: "Bu hafta" },
  { id: "lastWeek", label: "Geçen hafta" },
  { id: "days30", label: "Son 30 gün" },
];
const PERIOD_KEY = "kum.review.period";

/** Başka bir sayfadan verilen aralık (raporda bakılan gün/hafta/ay, zaman çizelgesindeki gün). */
export type ReviewRange = { start: string; days: number };

function rangeLabel({ start, days }: ReviewRange): string {
  const s = parseIsoDate(start);
  return days === 1 ? formatDate(s) : `${formatDate(s)} – ${formatDate(addDays(s, days - 1))}`;
}

function periodRange(p: Period, custom: ReviewRange | null): ReviewRange {
  const t = today();
  switch (p) {
    case "custom":
      return custom ?? { start: isoDate(startOfWeek(t)), days: 7 };
    case "today":
      return { start: isoDate(t), days: 1 };
    case "week":
      return { start: isoDate(startOfWeek(t)), days: 7 };
    case "lastWeek":
      return { start: isoDate(addDays(startOfWeek(t), -7)), days: 7 };
    case "days30":
      return { start: isoDate(addDays(t, -29)), days: 30 };
  }
}

function savedPeriod(): Period {
  try {
    const v = localStorage.getItem(PERIOD_KEY);
    if (PERIODS.some((p) => p.id === v)) return v as Period;
  } catch {
    /* depolama kapalı */
  }
  return "week";
}

/** Başlık satırlarından ilk bu kadarı açık gelir. */
const FIRST_ITEMS = 3;

/**
 * Gözden geçir: hiçbir projeye düşmeyen süre, site ve uygulamaya göre gruplanmış. Grup ya da
 * başlık tek tıkla projeye atanır; istenirse kural da eklenir (geçmişe ve geleceğe uygulanır,
 * etkisi önceden gösterilir). Her atama geri alınabilir.
 */
export default function Review({
  range,
  onOpenTimesheet,
  onOpenProjects,
}: {
  /** Verilirse sayfa bu aralıkla açılır; sekmelerden biri seçilince bırakılır. */
  range?: ReviewRange | null;
  onOpenTimesheet: () => void;
  onOpenProjects: () => void;
}) {
  const [period, setPeriodState] = useState<Period>(() => (range ? "custom" : savedPeriod()));
  const setPeriod = (p: Period) => {
    setPeriodState(p);
    try {
      if (p !== "custom") localStorage.setItem(PERIOD_KEY, p);
    } catch {
      /* depolama kapalı */
    }
  };
  const { start, days } = periodRange(period, range ?? null);
  const [data, setData] = useState<Unassigned | null>(null);
  const [worked, setWorked] = useState(0);
  const [tags, setTags] = useState<Tag[]>([]);
  const [pending, setPending] = useState<string[]>([]);
  const [ignored, setIgnored] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  // Dönem hızlı değişince yavaş kalan eski yanıt yenisinin üstüne yazmasın: atama da
  // ekrandaki grubun anahtarını şimdiki aralıkla gönderir.
  const seq = useRef(0);
  const shown = useRef("");
  const load = useCallback(() => {
    const n = ++seq.current;
    const key = `${start}/${days}`;
    if (shown.current !== key) setData(null);
    api.unassigned(start, days).then(
      (u) => {
        if (n !== seq.current) return;
        shown.current = key;
        setData(u);
        setError(null);
      },
      (e) => n === seq.current && setError(friendlyError(e)),
    );
    api.report(start, days, false).then(
      (r) => {
        if (n !== seq.current) return;
        setWorked(r.totalSeconds);
        setTags(r.tags);
      },
      () => {},
    );
    api.ignoredUnassigned().then(setIgnored, () => {});
  }, [start, days]);
  useEffect(load, [load]);
  useChanged(load);
  useEffect(() => {
    api.pendingTimesheetDays().then(setPending, () => setPending([]));
  }, []);

  const projects = useMemo(() => tags.filter((t) => t.kind === "project"), [tags]);

  async function assign(
    group: UnassignedGroup,
    projectId: string,
    rule: [RuleField, string] | null,
    title: string | null,
  ) {
    const name = projectId === NO_PROJECT ? "Projesiz" : (projects.find((p) => p.id === projectId)?.name ?? "proje");
    const what = title ? `“${short(title || "(başlıksız)")}”` : group.label;
    try {
      await undoable(
        api.assignUnassigned(start, days, group.key, title, projectId, rule),
        projectId === NO_PROJECT
          ? `${what} projesiz sayıldı`
          : rule
            ? `${what} → ${name}, kural eklendi`
            : `${what} → ${name}`,
      );
      notifyChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  async function ignore(group: UnassignedGroup, value: boolean) {
    try {
      await api.ignoreUnassigned(group.key, value);
      load();
      if (value)
        toast(`${group.label} listede gösterilmeyecek`, {
          action: { label: "Geri al", run: () => api.ignoreUnassigned(group.key, false).then(load) },
        });
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  const total = data ? data.totalSeconds : 0;
  const assignedShare = worked > 0 ? Math.max(0, Math.min(1, (worked - total) / worked)) : 0;

  return (
    <div className="mx-auto w-full max-w-4xl space-y-5 px-6 pt-1 pb-12">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="max-w-xl text-[13px] text-muted-foreground">
          Hiçbir projeye düşmeyen süre. Bir grubu ya da başlığı projeye ata; kural eklersen geçmişteki ve gelecekteki
          benzer süre de o projeye yazılır.
        </p>
        <Tabs value={period} onValueChange={(v) => setPeriod(v as Period)}>
          <TabsList aria-label="Dönem">
            {range && (
              <TabsTrigger value="custom" className="px-3">
                {rangeLabel(range)}
              </TabsTrigger>
            )}
            {PERIODS.map((p) => (
              <TabsTrigger key={p.id} value={p.id} className="px-3">
                {p.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>

      {error && <p className="px-1 text-xs text-destructive selectable">{error}</p>}

      {!data ? (
        <ReviewSkeleton />
      ) : (
        <>
          <Hero
            total={total}
            worked={worked}
            share={assignedShare}
            idle={data.idleSeconds}
            pending={pending.length}
            onOpenTimesheet={onOpenTimesheet}
          />

          {projects.length === 0 && (
            <div className="flex items-center gap-3 rounded-xl border border-dashed px-4 py-3 text-[13px]">
              <FolderKanban className="size-5 shrink-0 text-primary" />
              <p className="flex-1 text-muted-foreground">Süreyi atamak için önce bir proje ekle.</p>
              <Button size="sm" onClick={onOpenProjects}>
                Proje ekle
              </Button>
            </div>
          )}

          {data.groups.length === 0 && data.idle.length === 0 ? (
            <AllDone />
          ) : (
            <div className="space-y-3">
              {data.groups.map((g, i) => (
                <GroupCard
                  key={g.key}
                  group={g}
                  index={i}
                  total={total}
                  projects={projects}
                  tags={tags}
                  onAssign={(projectId, rule, title) => assign(g, projectId, rule, title)}
                  onIgnore={() => ignore(g, true)}
                />
              ))}
            </div>
          )}

          {data.idle.length > 0 && <IdleList idle={data.idle} projects={projects} />}

          {ignored.length > 0 && (
            <IgnoredList
              keys={ignored}
              onShow={(key) =>
                api.ignoreUnassigned(key, false).then(load, (e) => toast(friendlyError(e), { tone: "error" }))
              }
            />
          )}
        </>
      )}
    </div>
  );
}

function short(s: string, n = 40) {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}

/** Üstte özet: atanmamış süre, atanan payı halkası, boşta süre ve aktarılmamış günler. */
function Hero({
  total,
  worked,
  share,
  idle,
  pending,
  onOpenTimesheet,
}: {
  total: number;
  worked: number;
  share: number;
  idle: number;
  pending: number;
  onOpenTimesheet: () => void;
}) {
  return (
    <div className="hero-surface relative overflow-hidden rounded-2xl border p-5 shadow-sm">
      <div className="relative flex flex-wrap items-center gap-6">
        <Ring value={share} />
        <div className="min-w-0 flex-1 space-y-1">
          <div className="text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
            Projeye atanmamış
          </div>
          <div className="text-[34px] leading-none font-semibold tracking-tight tabular">{formatDuration(total)}</div>
          <p className="text-xs text-muted-foreground">
            {worked > 0
              ? `${formatDuration(worked)} çalışmanın %${Math.round(share * 100)}'i projelerde.`
              : "Bu dönemde takip edilen çalışma yok."}
          </p>
        </div>
        <div className="flex flex-col gap-2">
          {idle > 0 && (
            <a
              href="#bosta"
              className="flex items-center gap-2 rounded-lg border bg-background/70 px-3 py-1.5 text-xs hover:bg-accent"
            >
              <Coffee className="size-3.5 text-amber-500" />
              <span>
                Boşta <b className="font-semibold tabular">{formatDuration(idle)}</b>
              </span>
            </a>
          )}
          {pending > 0 && (
            <button
              onClick={onOpenTimesheet}
              className="flex items-center gap-2 rounded-lg border bg-background/70 px-3 py-1.5 text-left text-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <FileSpreadsheet className="size-3.5 text-emerald-500" />
              <span>
                Bu hafta <b className="font-semibold tabular">{pending} gün</b> aktarılmadı
              </span>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/** Atanan payı gösteren halka. */
function Ring({ value }: { value: number }) {
  const r = 30;
  const c = 2 * Math.PI * r;
  const pct = Math.round(value * 100);
  return (
    <div className="relative grid size-[76px] shrink-0 place-items-center">
      <svg viewBox="0 0 76 76" className="absolute inset-0 -rotate-90">
        <defs>
          <linearGradient id="ring-grad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--brand-1)" />
            <stop offset="100%" stopColor="var(--brand-2)" />
          </linearGradient>
        </defs>
        <circle cx="38" cy="38" r={r} fill="none" stroke="var(--muted)" strokeWidth="7" />
        <circle
          cx="38"
          cy="38"
          r={r}
          fill="none"
          stroke="url(#ring-grad)"
          strokeWidth="7"
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - value)}
          className="transition-[stroke-dashoffset] duration-700 ease-out"
        />
      </svg>
      <div className="text-center leading-none">
        <div className="text-[15px] font-semibold tabular">%{pct}</div>
        <div className="mt-0.5 text-[9px] text-muted-foreground">atandı</div>
      </div>
    </div>
  );
}

function AllDone() {
  return (
    <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed px-6 py-12 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-gradient-to-br from-emerald-400 to-teal-500 text-white shadow-lg shadow-emerald-500/25">
        <CheckCircle2 className="size-7" />
      </div>
      <div>
        <p className="text-[15px] font-semibold">Her şey bir projede</p>
        <p className="mt-1 text-[13px] text-muted-foreground">Bu dönemde atanmamış süre yok.</p>
      </div>
    </div>
  );
}

function ReviewSkeleton() {
  return (
    <div className="space-y-3">
      <div className="skeleton h-[118px] rounded-2xl" />
      {[0, 1, 2].map((i) => (
        <div key={i} className="skeleton h-[92px] rounded-xl" style={{ animationDelay: `${i * 120}ms` }} />
      ))}
    </div>
  );
}

/** Kural türüne göre metin. */
const RULE_TEXT: Record<RuleField, string> = {
  domain: "Bu siteyi hep bu projeye yaz",
  app: "Bu uygulamayı hep bu projeye yaz",
  title: "Başlığında şu geçenleri hep bu projeye yaz",
};

function GroupCard({
  group,
  index,
  total,
  projects,
  tags,
  onAssign,
  onIgnore,
}: {
  group: UnassignedGroup;
  index: number;
  total: number;
  projects: Tag[];
  tags: Tag[];
  onAssign: (projectId: string, rule: [RuleField, string] | null, title: string | null) => Promise<void>;
  onIgnore: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [menu, setMenu] = useState(false);
  const likely = projects.find((p) => p.id === group.likelyProject);
  const share = total > 0 ? group.seconds / total : 0;
  const items = expanded ? group.items : group.items.slice(0, FIRST_ITEMS);
  const Icon = group.kind === "site" ? Globe : AppWindow;
  const field: RuleField = group.kind === "site" ? "domain" : "app";

  return (
    <section
      className="group/card animate-in rounded-xl border bg-card shadow-xs transition-shadow duration-300 fade-in-0 slide-in-from-bottom-1 fill-mode-both hover:shadow-md"
      style={{ animationDelay: `${Math.min(index, 8) * 40}ms` }}
    >
      <div className="flex items-center gap-3 px-4 pt-3.5 pb-3">
        <div
          className={cn(
            "grid size-9 shrink-0 place-items-center rounded-lg",
            group.kind === "site"
              ? "bg-sky-500/12 text-sky-600 dark:text-sky-400"
              : "bg-violet-500/12 text-violet-600 dark:text-violet-400",
          )}
        >
          <Icon className="size-[18px]" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-semibold" title={group.label}>
            {group.label}
          </div>
          <div className="truncate text-[11px] text-muted-foreground">
            {group.kind === "site" ? group.appName : "Uygulama"} · {group.items.length + group.more} pencere
          </div>
        </div>
        <div className="w-28 shrink-0 text-right">
          <div className="text-[13px] font-semibold tabular">{formatDuration(group.seconds)}</div>
          <div className="mt-1 h-1 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-gradient-to-r from-[var(--brand-1)] to-[var(--brand-2)]"
              style={{ width: `${Math.max(4, share * 100)}%` }}
            />
          </div>
        </div>
        <Popover open={menu} onOpenChange={setMenu}>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Diğer seçenekler" className="text-muted-foreground">
              <MoreHorizontal />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-56 p-1">
            <MenuButton
              onClick={() => {
                setMenu(false);
                onAssign(NO_PROJECT, null, null);
              }}
            >
              <CheckCircle2 /> Projesiz say
              <span className="ml-auto text-[10px] text-muted-foreground">bu dönem</span>
            </MenuButton>
            <MenuButton
              onClick={() => {
                setMenu(false);
                onIgnore();
              }}
            >
              <EyeOff /> Listede hiç gösterme
            </MenuButton>
          </PopoverContent>
        </Popover>
      </div>

      <div className="space-y-2.5 border-t px-4 py-3">
        {likely && (
          <div className="flex flex-wrap items-center gap-2 rounded-lg bg-gradient-to-r from-primary/8 to-transparent px-2.5 py-1.5 text-xs">
            <Sparkles className="size-3.5 text-primary" />
            <span className="text-muted-foreground">Bu süre</span>
            <span className="inline-flex items-center gap-1.5 font-medium">
              <i className="size-2 rounded-full" style={{ background: tagColor(likely) }} />
              {likely.name}
            </span>
            <span className="text-muted-foreground">üzerinde çalışırken geçmiş; aşağıda seçili.</span>
          </div>
        )}
        <AssignForm
          projects={projects}
          tags={tags}
          field={field}
          pattern={group.pattern}
          editable={group.kind === "site"}
          defaultRule={false}
          ruleText={RULE_TEXT[field]}
          initialProject={likely?.id}
          submitLabel="Hepsini ata"
          onSubmit={(p, rule) => onAssign(p, rule, null)}
        />
      </div>

      <ul className="border-t">
        {items.map((item) => (
          <ItemRow
            key={item.title}
            item={item}
            projects={projects}
            tags={tags}
            likely={likely}
            onAssign={(p, rule) => onAssign(p, rule, item.title)}
          />
        ))}
      </ul>
      {group.items.length > FIRST_ITEMS && (
        <button
          className="flex w-full items-center justify-center gap-1 border-t py-2 text-[11px] font-medium text-muted-foreground outline-none hover:bg-accent/40 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
          onClick={() => setExpanded(!expanded)}
        >
          <ChevronDown className={cn("size-3.5 transition-transform", expanded && "rotate-180")} />
          {expanded ? "Daha az göster" : `${group.items.length - FIRST_ITEMS} başlık daha`}
        </button>
      )}
      {/* 15 dakikadan kısa başlıklar ayrı satır olmaz; liste açıkken ya da hiç uzun başlık yokken söylenir. */}
      {(expanded || group.items.length <= FIRST_ITEMS) && group.more > 0 && (
        <p className="border-t py-2 text-center text-[11px] text-muted-foreground">
          {group.items.length ? "ve " : ""}
          {group.more} kısa başlık (15 dk altı; grubu atamak hepsini kapsar)
        </p>
      )}
    </section>
  );
}

function MenuButton({ children, onClick }: { children: React.ReactNode; onClick: () => void }) {
  return (
    <button
      className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-xs outline-none hover:bg-accent focus-visible:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 [&_svg]:size-3.5 [&_svg]:text-muted-foreground"
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/**
 * Başlık satırı: süre ve "Ata"; açılınca proje ve başlık kuralı. Muhtemel proje biliniyorsa
 * tek tıkla ona atanır (kuralsız, geri alınabilir).
 */
function ItemRow({
  item,
  projects,
  tags,
  likely,
  onAssign,
}: {
  item: UnassignedItem;
  projects: Tag[];
  tags: Tag[];
  likely?: Tag;
  onAssign: (projectId: string, rule: [RuleField, string] | null) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  return (
    <li className="border-b last:border-b-0">
      <div className="flex items-center gap-2 px-4 py-2 pl-[3.75rem] hover:bg-accent/30">
        <span className={cn("min-w-0 flex-1 truncate text-xs", !item.title && "text-muted-foreground italic")}>
          {item.title || "(başlıksız)"}
        </span>
        {item.word && (
          <span className="hidden shrink-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary sm:inline-flex">
            <Wand2 className="size-3" /> {item.word}
          </span>
        )}
        <span className="w-14 shrink-0 text-right text-[11px] text-muted-foreground tabular">
          {formatDuration(item.seconds)}
        </span>
        {likely && !open && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 max-w-36 shrink-0 gap-1.5 px-2 text-[11px]"
            title={`${likely.name} projesine ata`}
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onAssign(likely.id, null);
              } finally {
                setBusy(false);
              }
            }}
          >
            <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(likely) }} aria-hidden />
            <span className="truncate">{likely.name}</span>
          </Button>
        )}
        <Button
          size="sm"
          variant={open ? "secondary" : "ghost"}
          className="h-7 shrink-0 px-2 text-[11px]"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          disabled={projects.length === 0}
        >
          {likely ? "Başka…" : "Ata"}
        </Button>
      </div>
      {open && (
        <div className="animate-in px-4 pb-3 pl-[3.75rem] fade-in-0 slide-in-from-top-1">
          <AssignForm
            projects={projects}
            tags={tags}
            field="title"
            pattern={item.word ?? item.title}
            editable
            defaultRule={!!item.word}
            ruleText={RULE_TEXT.title}
            initialProject={likely?.id}
            submitLabel="Ata"
            onSubmit={async (p, rule) => {
              await onAssign(p, rule);
              setOpen(false);
            }}
          />
        </div>
      )}
    </li>
  );
}

/** Proje seçimi, isteğe bağlı kural ve önizlemesi. */
function AssignForm({
  projects,
  tags,
  field,
  pattern: initialPattern,
  editable,
  defaultRule,
  ruleText,
  initialProject,
  submitLabel,
  onSubmit,
}: {
  projects: Tag[];
  tags: Tag[];
  field: RuleField;
  pattern: string;
  editable: boolean;
  defaultRule: boolean;
  ruleText: string;
  initialProject?: string;
  submitLabel: string;
  onSubmit: (projectId: string, rule: [RuleField, string] | null) => Promise<void>;
}) {
  const [project, setProject] = useState(initialProject ?? "");
  // Projeler atanmamış süreden sonra gelebilir: önerilen proje o zaman seçilsin.
  useEffect(() => {
    if (initialProject) setProject((p) => p || initialProject);
  }, [initialProject]);
  const [rule, setRule] = useState(defaultRule);
  const [pattern, setPattern] = useState(initialPattern);
  const [busy, setBusy] = useState(false);
  if (projects.length === 0) return null;
  const usable = !rule || pattern.trim().length > 0;
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <ProjectSelect value={project} onChange={setProject} projects={projects} className="w-48" />
        <label className="flex min-w-0 flex-1 items-center gap-2 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={rule}
            onChange={(e) => setRule(e.target.checked)}
            className="size-3.5 accent-[var(--primary)]"
          />
          <span className="shrink-0">{ruleText}</span>
          {rule &&
            (editable ? (
              <Input
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
                className="h-7 min-w-24 flex-1 text-xs"
                aria-label="Kural deseni"
              />
            ) : (
              <code className="truncate rounded bg-muted px-1.5 py-0.5 text-[11px]">{pattern}</code>
            ))}
        </label>
        <Button
          size="sm"
          disabled={!project || !usable || busy}
          onClick={async () => {
            setBusy(true);
            try {
              await onSubmit(project, rule ? [field, pattern.trim()] : null);
            } finally {
              setBusy(false);
            }
          }}
        >
          {submitLabel}
        </Button>
      </div>
      {rule && project && <RulePreview tagId={project} field={field} pattern={pattern} tags={tags} />}
    </div>
  );
}

const dayTime = (iso: string) => `${formatDate(new Date(iso))} ${formatTime(new Date(iso))}`;

/** Bilgisayardan uzakta geçen, projeye atanmamış aralıklar. */
function IdleList({ idle, projects }: { idle: { start: string; end: string; seconds: number }[]; projects: Tag[] }) {
  return (
    <section id="bosta" className="scroll-mt-4 space-y-2">
      <div className="flex items-center gap-2 px-1">
        <Coffee className="size-4 text-amber-500" />
        <h2 className="text-[13px] font-semibold">Bilgisayardan uzakta</h2>
        <span className="text-xs text-muted-foreground">
          Toplantı ya da yüz yüze iş idiyse projeye ata; çalışma süresine eklenir.
        </span>
      </div>
      <ul className="divide-y rounded-xl border bg-card shadow-xs">
        {idle.map((s) => (
          <li key={s.start} className="flex items-center gap-3 px-4 py-2">
            <CalendarClock className="size-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 text-xs tabular">
              {dayTime(s.start)} – {formatTime(new Date(s.end))}
            </span>
            <span className="w-16 text-right text-[11px] text-muted-foreground tabular">
              {formatDuration(s.seconds)}
            </span>
            {projects.length > 0 && (
              <ProjectSelect
                value=""
                placeholder="Projeye ata…"
                projects={projects}
                className="w-44"
                onChange={(id) =>
                  undoable(api.setRangeProject(s.start, s.end, id), "Boşta süre projeye atandı").then(
                    notifyChanged,
                    (e) => toast(friendlyError(e), { tone: "error" }),
                  )
                }
              />
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function IgnoredList({ keys, onShow }: { keys: string[]; onShow: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="px-1 text-xs text-muted-foreground">
      <button
        className="rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {keys.length} grup listede gösterilmiyor {open ? "▴" : "▾"}
      </button>
      {open && (
        <ul className="mt-2 flex flex-wrap gap-1.5">
          {keys.map((k) => (
            <li key={k} className="inline-flex items-center gap-1 rounded-md border bg-card py-0.5 pr-0.5 pl-2">
              <span>{k.replace(/^(site|app):/, "")}</span>
              <Button size="sm" variant="ghost" className="h-5 px-1.5 text-[11px]" onClick={() => onShow(k)}>
                Göster
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
