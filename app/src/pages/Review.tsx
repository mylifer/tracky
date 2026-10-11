import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AppWindow,
  CheckCircle2,
  ChevronDown,
  Coffee,
  EyeOff,
  FileSpreadsheet,
  FolderKanban,
  Globe,
  SkipForward,
  Sparkles,
  Wand2,
} from "lucide-react";
import {
  api,
  formatDuration,
  NO_PROJECT,
  type RuleField,
  type RuleSuggestion,
  type Tag,
  type Unassigned,
  type UnassignedGroup,
  type UnassignedItem,
} from "../api";
import { ProjectSelect } from "../components/ProjectSelect";
import { RulePreview } from "../components/RulePreview";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addDays, formatDate, formatTime, isoDate, parseIsoDate, startOfWeek, today } from "../lib/dates";
import { addSuggestedRule, friendlyError, notifyChanged, toast, undoable, useChanged } from "../lib/feedback";
import { readRecent, rememberRecent } from "../lib/projectOptions";
import { ruleReason, ruleSubject } from "../lib/ruleSuggestions";
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
/** "Bundan sonra da otomatik yaz" son seçimi. */
const RULE_KEY = "kum.review.rule";

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

function savedRule(): boolean {
  try {
    return localStorage.getItem(RULE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Kartta listelenen başlık sayısı. */
const CARD_ITEMS = 4;
/** Kartta numaralı (1–4) proje düğmesi sayısı. */
const CHOICES = 4;

/** Sıradaki tek kart: projeye düşmeyen bir site/uygulama grubu ya da boşta geçen bir aralık. */
type Entry =
  | { key: string; kind: "group"; group: UnassignedGroup; seconds: number }
  | { key: string; kind: "idle"; start: string; end: string; seconds: number };

/**
 * Gözden geçir: hiçbir projeye düşmeyen süre, sırayla tek kart. Her kart bir site/uygulama
 * grubu ya da boşta aralık; numaralı düğmeyle (1–4) projeye atanır, istenirse kural da eklenir
 * (geçmişe ve geleceğe uygulanır). Atla kartı sona alır. Her atama geri alınabilir.
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
    setSkipped([]);
    setDone(0);
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
  const [projectSeconds, setProjectSeconds] = useState<Record<string, number>>({});
  const [pending, setPending] = useState<string[]>([]);
  const [ignored, setIgnored] = useState<string[]>([]);
  const [suggestions, setSuggestions] = useState<RuleSuggestion[]>([]);
  const [error, setError] = useState<string | null>(null);
  /** Atlanan kartlar (sırayla sona gider). */
  const [skipped, setSkipped] = useState<string[]>([]);
  /** Bu dönemde bu sayfada atanan kart sayısı (sayaç için). */
  const [done, setDone] = useState(0);
  // Atanan kart yanıt beklenmeden gizlenir; atama bitince ve yeni liste gelince bırakılır
  // (geri alınınca kart yeniden görünsün).
  const hidden = useRef(new Map<string, boolean>());
  const [, redraw] = useState(0);

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
        for (const [k, settled] of hidden.current) if (settled) hidden.current.delete(k);
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
        setProjectSeconds(Object.fromEntries(r.projects.flatMap((b) => (b.id ? [[b.id, b.seconds]] : []))));
      },
      () => {},
    );
    api.ignoredUnassigned().then(setIgnored, () => {});
    // Kural önerileri dönemden bağımsızdır (son 30 günün elle atamaları).
    api.ruleSuggestions().then(setSuggestions, () => setSuggestions([]));
  }, [start, days]);
  useEffect(load, [load]);
  useChanged(load);
  useEffect(() => {
    api.pendingTimesheetDays().then(setPending, () => setPending([]));
  }, []);

  const projects = useMemo(() => tags.filter((t) => t.kind === "project"), [tags]);

  const queue = useMemo(() => {
    if (!data) return [];
    const all: Entry[] = [
      ...data.groups.map((g): Entry => ({ key: g.key, kind: "group", group: g, seconds: g.seconds })),
      ...data.idle.map((s): Entry => ({ key: `idle:${s.start}`, kind: "idle", ...s })),
    ]
      .filter((e) => !hidden.current.has(e.key))
      .sort((a, b) => b.seconds - a.seconds);
    const rank = (e: Entry) => skipped.indexOf(e.key);
    return [...all.filter((e) => rank(e) < 0), ...all.filter((e) => rank(e) >= 0).sort((a, b) => rank(a) - rank(b))];
    // `hidden` bir ref; değişince `redraw` yeniden hesaplatır.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, skipped, hidden.current.size]);
  const current = queue[0] ?? null;
  const left = queue.reduce((a, e) => a + e.seconds, 0);

  /** Kartı gizleyip işi yapar; hata olursa kart geri gelir. */
  async function settle(key: string, work: () => Promise<unknown>) {
    hidden.current.set(key, false);
    redraw((x) => x + 1);
    try {
      await work();
      hidden.current.set(key, true);
      setDone((d) => d + 1);
      setSkipped((s) => s.filter((k) => k !== key));
      notifyChanged();
    } catch (e) {
      hidden.current.delete(key);
      redraw((x) => x + 1);
      toast(friendlyError(e), { tone: "error" });
    }
  }

  function projectName(id: string) {
    return id === NO_PROJECT ? "Projesiz" : (projects.find((p) => p.id === id)?.name ?? "proje");
  }

  function assignEntry(entry: Entry, projectId: string, rule: [RuleField, string] | null) {
    if (projectId !== NO_PROJECT) rememberRecent(projectId);
    if (entry.kind === "idle")
      return settle(entry.key, () =>
        undoable(api.setRangeProject(entry.start, entry.end, projectId), `Boşta süre → ${projectName(projectId)}`),
      );
    const g = entry.group;
    return settle(entry.key, () =>
      undoable(
        api.assignUnassigned(start, days, g.key, null, projectId, rule),
        projectId === NO_PROJECT
          ? `${g.label} projesiz sayıldı`
          : rule
            ? `${g.label} → ${projectName(projectId)}, kural eklendi`
            : `${g.label} → ${projectName(projectId)}`,
      ),
    );
  }

  /** Grubun tek başlığı; kart yerinde kalır, liste yenilenince başlık düşer. */
  async function assignTitle(g: UnassignedGroup, title: string, projectId: string, rule: [RuleField, string] | null) {
    rememberRecent(projectId);
    const what = `“${short(title || "(başlıksız)")}”`;
    try {
      await undoable(
        api.assignUnassigned(start, days, g.key, title, projectId, rule),
        rule ? `${what} → ${projectName(projectId)}, kural eklendi` : `${what} → ${projectName(projectId)}`,
      );
      notifyChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  async function ignore(key: string, label: string) {
    await settle(key, () => api.ignoreUnassigned(key, true));
    toast(`${label} listede gösterilmeyecek`, {
      action: {
        label: "Geri al",
        run: () => api.ignoreUnassigned(key, false).then(load, (e) => toast(friendlyError(e), { tone: "error" })),
      },
    });
  }

  function skip(key: string) {
    setSkipped((s) => [...s.filter((k) => k !== key), key]);
  }

  async function dismissSuggestion(s: RuleSuggestion) {
    setSuggestions((list) => list.filter((x) => x.key !== s.key));
    try {
      await api.dismissRuleSuggestion(s.key);
      toast(`“${ruleSubject(s)}” bir daha önerilmeyecek`);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
      load();
    }
  }

  const share = worked > 0 ? Math.max(0, Math.min(1, (worked - left) / worked)) : 0;

  return (
    <div className="mx-auto w-full max-w-2xl space-y-5 px-6 pt-1 pb-12">
      <div className="flex flex-wrap items-center justify-end gap-3">
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
          <Progress left={left} worked={worked} share={share} />

          {projects.length === 0 ? (
            <div className="flex items-center gap-3 rounded-xl border border-dashed px-4 py-3 text-[13px]">
              <FolderKanban className="size-5 shrink-0 text-primary" />
              <p className="flex-1 text-muted-foreground">Süreyi atamak için önce bir proje ekle.</p>
              <Button size="sm" onClick={onOpenProjects}>
                Proje ekle
              </Button>
            </div>
          ) : current ? (
            <div className="space-y-3">
              <p className="text-center text-xs text-muted-foreground tabular">
                <b className="font-semibold text-foreground">{done + 1}</b> / {done + queue.length}
              </p>
              <ReviewCard
                key={current.key}
                entry={current}
                projects={projects}
                tags={tags}
                projectSeconds={projectSeconds}
                onAssign={(p, rule) => assignEntry(current, p, rule)}
                onAssignTitle={(title, p, rule) =>
                  current.kind === "group" && assignTitle(current.group, title, p, rule)
                }
                onSkip={queue.length > 1 ? () => skip(current.key) : undefined}
                onIgnore={current.kind === "group" ? () => ignore(current.key, current.group.label) : undefined}
              />
            </div>
          ) : (
            <AllDone />
          )}

          {pending.length > 0 && (
            <button
              onClick={onOpenTimesheet}
              className="mx-auto flex items-center gap-2 rounded-lg px-3 py-1.5 text-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              <FileSpreadsheet className="size-3.5 text-emerald-500" />
              Bu hafta <b className="font-semibold tabular">{pending.length} gün</b> zaman çizelgesine aktarılmadı
            </button>
          )}

          {suggestions.length > 0 && projects.length > 0 && (
            <RuleSuggestions
              suggestions={suggestions}
              projects={projects}
              tags={tags}
              onAdd={async (s) => {
                if (await addSuggestedRule(s)) setSuggestions((list) => list.filter((x) => x.key !== s.key));
              }}
              onDismiss={dismissSuggestion}
            />
          )}

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

/** Tek satır özet: kalan atanmamış süre ve projelerdeki pay. */
function Progress({ left, worked, share }: { left: number; worked: number; share: number }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="shrink-0">
        <b className="font-semibold text-foreground tabular">{formatDuration(left)}</b> atanmamış
      </span>
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-gradient-to-r from-[var(--brand-1)] to-[var(--brand-2)] transition-[width] duration-500 ease-out"
          style={{ width: `${share * 100}%` }}
        />
      </div>
      <span className="shrink-0 tabular">{worked > 0 ? `%${Math.round(share * 100)} projede` : "çalışma yok"}</span>
    </div>
  );
}

/**
 * Numaralı düğmeler: önce muhtemel proje (öncesinde/sonrasında çalışılan), sonra son seçilenler,
 * sonra dönemde en çok süre alan projeler.
 */
function pickChoices(projects: Tag[], likely: string | null, projectSeconds: Record<string, number>): Tag[] {
  const byId = new Map(projects.filter((p) => !p.archived).map((p) => [p.id, p]));
  const ranked = [...byId.values()].sort((a, b) => (projectSeconds[b.id] ?? 0) - (projectSeconds[a.id] ?? 0));
  const ids = [...(likely ? [likely] : []), ...readRecent(), ...ranked.map((p) => p.id)];
  return [...new Set(ids)].flatMap((id) => byId.get(id) ?? []).slice(0, CHOICES);
}

/** Kural türüne göre metin. */
const RULE_TEXT: Record<RuleField, string> = {
  domain: "Bu siteyi bundan sonra da bu projeye yaz",
  app: "Bu uygulamayı bundan sonra da bu projeye yaz",
  title: "Başlığında şu geçenleri hep bu projeye yaz",
};

const dayTime = (iso: string) => `${formatDate(new Date(iso))} ${formatTime(new Date(iso))}`;

function ReviewCard({
  entry,
  projects,
  tags,
  projectSeconds,
  onAssign,
  onAssignTitle,
  onSkip,
  onIgnore,
}: {
  entry: Entry;
  projects: Tag[];
  tags: Tag[];
  projectSeconds: Record<string, number>;
  onAssign: (projectId: string, rule: [RuleField, string] | null) => Promise<void>;
  onAssignTitle: (title: string, projectId: string, rule: [RuleField, string] | null) => void;
  onSkip?: () => void;
  onIgnore?: () => void;
}) {
  const group = entry.kind === "group" ? entry.group : null;
  const likely = group?.likelyProject ?? null;
  const choices = useMemo(
    () => pickChoices(projects, likely, projectSeconds),
    // Kart açıldığında sabitlenir: atama "son seçilenler"i değiştirince düğmeler kaymasın.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entry.key, projects],
  );
  const field: RuleField = group?.kind === "site" ? "domain" : "app";
  const [rule, setRuleState] = useState(savedRule);
  const setRule = (v: boolean) => {
    setRuleState(v);
    try {
      localStorage.setItem(RULE_KEY, v ? "1" : "0");
    } catch {
      /* depolama kapalı */
    }
  };
  const [pattern, setPattern] = useState(group?.pattern ?? "");
  const [previewFor, setPreviewFor] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const ruleOn = !!group && rule && pattern.trim().length > 0;

  async function choose(projectId: string) {
    if (busy) return;
    setBusy(true);
    try {
      await onAssign(projectId, ruleOn && projectId !== NO_PROJECT ? [field, pattern.trim()] : null);
    } finally {
      setBusy(false);
    }
  }

  // 1–4 projeye atar, S atlar. Uygulamanın 1/2/3 kısayollarından önce yakalanır.
  const keys = useRef<((e: KeyboardEvent) => void) | null>(null);
  keys.current = (e) => {
    if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]")) return;
    const n = Number(e.key);
    if (n >= 1 && n <= choices.length) void choose(choices[n - 1].id);
    else if ((e.key === "s" || e.key === "S") && onSkip) onSkip();
    else return;
    e.preventDefault();
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current?.(e);
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const Icon = entry.kind === "idle" ? Coffee : group?.kind === "site" ? Globe : AppWindow;
  const items = group ? (expanded ? group.items : group.items.slice(0, CARD_ITEMS)) : [];
  const likelyTag = projects.find((p) => p.id === likely);
  const preview = previewFor ?? choices[0]?.id;

  return (
    <section className="animate-in space-y-4 rounded-2xl border bg-card p-5 shadow-sm fade-in-0 slide-in-from-right-3">
      <div className="flex items-center gap-3">
        <div
          className={cn(
            "grid size-10 shrink-0 place-items-center rounded-xl",
            entry.kind === "idle"
              ? "bg-amber-500/14 text-amber-600 dark:text-amber-400"
              : group?.kind === "site"
                ? "bg-sky-500/12 text-sky-600 dark:text-sky-400"
                : "bg-violet-500/12 text-violet-600 dark:text-violet-400",
          )}
        >
          <Icon className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] font-semibold" title={group?.label}>
            {group ? group.label : "Bilgisayardan uzakta"}
          </div>
          <div className="truncate text-xs text-muted-foreground tabular">
            {entry.kind === "idle"
              ? `${dayTime(entry.start)} – ${formatTime(new Date(entry.end))} · toplantı ya da yüz yüze iş miydi?`
              : `${group!.kind === "site" ? group!.appName : "Uygulama"} · ${group!.items.length + group!.more} pencere`}
          </div>
        </div>
        <div className="shrink-0 text-[22px] font-semibold tracking-tight tabular">{formatDuration(entry.seconds)}</div>
      </div>

      {likelyTag && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Sparkles className="size-3.5 text-primary" />
          Öncesinde ve sonrasında
          <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2 py-0.5 font-medium text-foreground">
            <i className="size-2 rounded-full" style={{ background: tagColor(likelyTag) }} aria-hidden />
            {likelyTag.name}
          </span>
          üzerinde çalışıyordun.
        </p>
      )}

      {group && (items.length > 0 || group.more > 0) && (
        <ul className="-mx-2 border-t pt-2">
          {items.map((item) => (
            <TitleRow
              key={item.title}
              item={item}
              projects={projects}
              tags={tags}
              likely={likely ?? undefined}
              onAssign={(p, r) => onAssignTitle(item.title, p, r)}
            />
          ))}
          {group.items.length > CARD_ITEMS && (
            <li>
              <button
                className="flex w-full items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                onClick={() => setExpanded(!expanded)}
              >
                <ChevronDown className={cn("size-3.5 transition-transform", expanded && "rotate-180")} />
                {expanded ? "Daha az göster" : `${group.items.length - CARD_ITEMS} başlık daha`}
              </button>
            </li>
          )}
          {group.more > 0 && (expanded || group.items.length <= CARD_ITEMS) && (
            <li className="px-2 py-1 text-[11px] text-muted-foreground">
              {group.items.length ? "ve " : ""}
              {group.more} kısa pencere (15 dk altı; kart atanınca hepsi dahil)
            </li>
          )}
        </ul>
      )}

      <div className="grid grid-cols-2 gap-2">
        {choices.map((p, i) => (
          <button
            key={p.id}
            disabled={busy}
            onClick={() => choose(p.id)}
            onMouseEnter={() => setPreviewFor(p.id)}
            onFocus={() => setPreviewFor(p.id)}
            className={cn(
              "flex h-10 min-w-0 items-center gap-2 rounded-xl border px-3 text-left text-[13px] font-medium outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 disabled:opacity-50",
              p.id === likely && "border-primary/60 bg-primary/8 hover:bg-primary/12",
            )}
          >
            <i className="size-2.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} aria-hidden />
            <span className="min-w-0 flex-1 truncate">{p.name}</span>
            <kbd className="rounded border border-b-2 bg-background px-1.5 font-mono text-[10px] text-muted-foreground">
              {i + 1}
            </kbd>
          </button>
        ))}
        {projects.length > choices.length && (
          <ProjectSelect
            value=""
            placeholder="Başka proje…"
            projects={projects}
            className="col-span-2 h-9"
            onChange={(id) => id && void choose(id)}
          />
        )}
      </div>

      {group && (
        <div className="space-y-2">
          <label className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={rule}
              onChange={(e) => setRule(e.target.checked)}
              className="size-3.5 accent-[var(--primary)]"
            />
            <span className="shrink-0">{RULE_TEXT[field]}</span>
            {rule &&
              (group.kind === "site" ? (
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
          {ruleOn && preview && <RulePreview tagId={preview} field={field} pattern={pattern} tags={tags} />}
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3">
        <div className="flex items-center gap-1">
          {group && (
            <Button
              size="sm"
              variant="ghost"
              className="text-muted-foreground"
              disabled={busy}
              onClick={() => choose(NO_PROJECT)}
            >
              <CheckCircle2 /> Projesiz say
            </Button>
          )}
          {onIgnore && (
            <Button size="sm" variant="ghost" className="text-muted-foreground" disabled={busy} onClick={onIgnore}>
              <EyeOff /> Hiç gösterme
            </Button>
          )}
        </div>
        {onSkip && (
          <Button size="sm" variant="outline" disabled={busy} onClick={onSkip}>
            <SkipForward /> Atla
            <kbd className="ml-1 font-mono text-[10px] text-muted-foreground">S</kbd>
          </Button>
        )}
      </div>
    </section>
  );
}

/**
 * Karttaki başlık: süre ve "Ayrı ata". Kart tümüyle atanmadan önce bir başlık başka projeye
 * (istenirse başlık kuralıyla) ayrılabilir.
 */
function TitleRow({
  item,
  projects,
  tags,
  likely,
  onAssign,
}: {
  item: UnassignedItem;
  projects: Tag[];
  tags: Tag[];
  likely?: string;
  onAssign: (projectId: string, rule: [RuleField, string] | null) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <li>
      <div className="group/title flex items-center gap-2 rounded-md px-2 py-1 hover:bg-accent/40">
        <span className={cn("min-w-0 flex-1 truncate text-xs", !item.title && "text-muted-foreground italic")}>
          {item.title || "(başlıksız)"}
        </span>
        {item.word && (
          <span className="hidden shrink-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary sm:inline-flex">
            <Wand2 className="size-3" /> {item.word}
          </span>
        )}
        <button
          className={cn(
            "shrink-0 rounded px-1.5 text-[11px] text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/50",
            !open && "opacity-0 group-hover/title:opacity-100 focus-visible:opacity-100",
          )}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? "Vazgeç" : "Ayrı ata"}
        </button>
        <span className="w-14 shrink-0 text-right text-[11px] text-muted-foreground tabular">
          {formatDuration(item.seconds)}
        </span>
      </div>
      {open && (
        <div className="animate-in px-2 pt-1 pb-2 fade-in-0 slide-in-from-top-1">
          <AssignForm
            projects={projects}
            tags={tags}
            field="title"
            pattern={item.word ?? item.title}
            editable
            defaultRule={!!item.word}
            ruleText={RULE_TEXT.title}
            initialProject={likely}
            submitLabel="Ata"
            onSubmit={async (p, rule) => {
              onAssign(p, rule);
              setOpen(false);
            }}
          />
        </div>
      )}
    </li>
  );
}

function ReviewSkeleton() {
  return (
    <div className="space-y-3">
      <div className="skeleton h-4 rounded-full" />
      <div className="skeleton h-[320px] rounded-2xl" />
    </div>
  );
}

/**
 * Elle atamalardan öğrenilen kural önerileri: her biri gerekçesi ve önizlemesiyle; "Kural ekle"
 * kuralı ekler (geri alınabilir), "Önerme" bir daha göstermez.
 */
function RuleSuggestions({
  suggestions,
  projects,
  tags,
  onAdd,
  onDismiss,
}: {
  suggestions: RuleSuggestion[];
  projects: Tag[];
  tags: Tag[];
  onAdd: (s: RuleSuggestion) => Promise<void>;
  onDismiss: (s: RuleSuggestion) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <section className="space-y-2">
      <button
        className="mx-auto flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <Wand2 className="size-3.5 text-primary" />
        <b className="font-semibold">{suggestions.length} kural önerisi</b>: sık elle atadığın süre kendiliğinden düşsün
        <ChevronDown className={cn("size-3.5 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <ul className="space-y-2">
          {suggestions.map((s) => (
            <RuleSuggestionCard
              key={s.key}
              suggestion={s}
              project={projects.find((p) => p.id === s.projectId)}
              tags={tags}
              onAdd={() => onAdd(s)}
              onDismiss={() => onDismiss(s)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function RuleSuggestionCard({
  suggestion: s,
  project,
  tags,
  onAdd,
  onDismiss,
}: {
  suggestion: RuleSuggestion;
  project?: Tag;
  tags: Tag[];
  onAdd: () => Promise<void>;
  onDismiss: () => void;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <li className="animate-in space-y-2 rounded-xl border bg-card px-4 py-3 shadow-xs fade-in-0 slide-in-from-bottom-1">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5 text-[13px] font-semibold">
            <span>{ruleSubject(s)}</span>
            <span className="text-muted-foreground">→</span>
            <span className="inline-flex items-center gap-1.5">
              {project && <i className="size-2 rounded-full" style={{ background: tagColor(project) }} aria-hidden />}
              {project?.name ?? s.projectName}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] text-muted-foreground">{ruleReason(s)}</p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button size="sm" variant="ghost" className="text-muted-foreground" onClick={onDismiss} disabled={busy}>
            Önerme
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onAdd();
              } finally {
                setBusy(false);
              }
            }}
          >
            Kural ekle
          </Button>
        </div>
      </div>
      <RulePreview tagId={s.projectId} field={s.field} pattern={s.pattern} tags={tags} />
    </li>
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
