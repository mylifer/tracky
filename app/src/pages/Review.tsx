import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FileSpreadsheet, FolderKanban } from "lucide-react";
import {
  api,
  NO_PROJECT,
  type RuleField,
  type RuleSuggestion,
  type Tag,
  type Unassigned,
  type UnassignedGroup,
} from "../api";
import { Button } from "../components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addDays, formatDate, isoDate, parseIsoDate, startOfWeek, today } from "../lib/dates";
import { addSuggestedRule, friendlyError, notifyChanged, toast, undoable, useChanged } from "../lib/feedback";
import { rememberRecent } from "../lib/projectOptions";
import { ruleSubject } from "../lib/ruleSuggestions";
import { useLatest } from "../lib/latest";
import { ReviewCard } from "./review/ReviewCard";
import { AllDone, IgnoredList, Progress, ReviewSkeleton, RuleSuggestions, SkippedList } from "./review/parts";

type Period = "today" | "week" | "lastWeek" | "days30" | "custom";
const PERIODS: { id: Period; label: string }[] = [
  { id: "today", label: "Bugün" },
  { id: "week", label: "Bu hafta" },
  { id: "lastWeek", label: "Geçen hafta" },
  { id: "days30", label: "Son 30 gün" },
];
const PERIOD_KEY = "kum.review.period";
/** Atlanan gruplar (`site:alan`, `app:kimlik`): kartta bir daha gelmez, altta listelenir. */
const SKIPPED_KEY = "kum.review.skipped";

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

function savedSkipped(): string[] {
  try {
    const list: unknown = JSON.parse(localStorage.getItem(SKIPPED_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function saveSkipped(keys: string[]) {
  try {
    localStorage.setItem(SKIPPED_KEY, JSON.stringify(keys));
  } catch {
    /* depolama kapalıysa bu oturumla sınırlı kalır */
  }
}

/**
 * Gözden geçir: hiçbir projeye düşmeyen süre, sırayla tek kart. Her kart bir site/uygulama
 * grubu; numaralı düğmeyle (1–4) projeye atanır, istenirse kural da eklenir (geçmişe ve geleceğe
 * uygulanır). Atlanan grup kartta bir daha gelmez, altta "Atlananlar"da durur. Boşta geçen süre
 * burada önerilmez (takvimden atanır). Her atama geri alınabilir.
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
  const [skipped, setSkippedState] = useState<string[]>(savedSkipped);
  const setSkipped = (f: (keys: string[]) => string[]) =>
    setSkippedState((keys) => {
      const next = f(keys);
      saveSkipped(next);
      return next;
    });
  /** Bu dönemde bu sayfada atanan kart sayısı (sayaç için). */
  const [done, setDone] = useState(0);
  // Atanan kart yanıt beklenmeden gizlenir; atama bitince ve yeni liste gelince bırakılır
  // (geri alınınca kart yeniden görünsün).
  const hidden = useRef(new Map<string, boolean>());
  const [, redraw] = useState(0);

  // Dönem hızlı değişince yavaş kalan eski yanıt yenisinin üstüne yazmasın: atama da
  // ekrandaki grubun anahtarını şimdiki aralıkla gönderir.
  const begin = useLatest();
  const shown = useRef("");
  const load = useCallback(() => {
    const current = begin();
    const key = `${start}/${days}`;
    if (shown.current !== key) setData(null);
    api.unassigned(start, days).then(
      (u) => {
        if (!current()) return;
        shown.current = key;
        for (const [k, settled] of hidden.current) if (settled) hidden.current.delete(k);
        setData(u);
        setError(null);
      },
      (e) => current() && setError(friendlyError(e)),
    );
    api.report(start, days, false).then(
      (r) => {
        if (!current()) return;
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
    return data.groups.filter((g) => !hidden.current.has(g.key)).sort((a, b) => b.seconds - a.seconds);
    // `hidden` bir ref; değişince `redraw` yeniden hesaplatır.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, hidden.current.size]);
  const waiting = queue.filter((g) => !skipped.includes(g.key));
  const skippedGroups = queue.filter((g) => skipped.includes(g.key));
  const current = waiting[0] ?? null;
  const left = queue.reduce((a, g) => a + g.seconds, 0);

  /** Kartı gizleyip işi yapar; hata olursa kart geri gelir. */
  async function settle(key: string, work: () => Promise<unknown>) {
    hidden.current.set(key, false);
    redraw((x) => x + 1);
    try {
      await work();
      hidden.current.set(key, true);
      setDone((d) => d + 1);
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

  function assignGroup(g: UnassignedGroup, projectId: string, rule: [RuleField, string] | null) {
    if (projectId !== NO_PROJECT) rememberRecent(projectId);
    return settle(g.key, () =>
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
    setSkipped((keys) => [...keys.filter((k) => k !== key), key]);
  }

  function unskip(key: string) {
    setSkipped((keys) => keys.filter((k) => k !== key));
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
                <b className="font-semibold text-foreground">{done + 1}</b> / {done + waiting.length}
              </p>
              <ReviewCard
                key={current.key}
                group={current}
                projects={projects}
                tags={tags}
                projectSeconds={projectSeconds}
                onAssign={(p, rule) => assignGroup(current, p, rule)}
                onAssignTitle={(title, p, rule) => assignTitle(current, title, p, rule)}
                onSkip={() => skip(current.key)}
                onIgnore={() => ignore(current.key, current.label)}
              />
            </div>
          ) : (
            <AllDone skipped={skippedGroups.length} />
          )}

          {skippedGroups.length > 0 && projects.length > 0 && (
            <SkippedList
              groups={skippedGroups}
              projects={projects}
              onAssign={(g, p) => assignGroup(g, p, null)}
              onRestore={unskip}
            />
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
