import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  CalendarDays,
  ChevronRight,
  FileSpreadsheet,
  FolderKanban,
  Inbox,
  Pause,
  Play,
  Tags,
  Wallet,
} from "lucide-react";
import {
  api,
  type Budgets,
  formatDuration,
  type ProjectGoal,
  type Report,
  type Suggestions,
  type TrackingStatus,
  type Unassigned,
} from "../api";
import Toolbar from "../components/Toolbar";
import { GoalRing } from "../components/Summary";
import { HATCH } from "../components/Calendar";
import { Button } from "../components/ui/button";
import { Card, CardContent } from "../components/ui/card";
import { budgetRatio, budgetState, formatDays } from "../lib/budget";
import { addDays, formatTime, isoDate, parseIsoDate, startOfWeek, today } from "../lib/dates";
import { friendlyError, useChanged } from "../lib/feedback";
import { tagColor, tagMap, UNASSIGNED } from "../lib/tags";
import { UNASSIGNED_MIN } from "../lib/timesheet";
import { useTauriEvent } from "../lib/useTauriEvent";
import { cn } from "../lib/utils";

/** Sayfa açıkken rakamların yenilenme aralığı (takip sürerken süre artar). */
const REFRESH_MS = 60_000;

const dayName = new Intl.DateTimeFormat("tr-TR", { weekday: "long" });
const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long" });

type Data = {
  report: Report;
  /** Dünün bu saate kadarki raporu (kıyas için). */
  yesterday: Report | null;
  week: Report | null;
  unassigned: Unassigned | null;
  pending: string[];
  suggestions: Suggestions | null;
  budgets: Budgets | null;
  dailyHours: number;
  projectGoals: ProjectGoal[];
};

type Props = {
  tracking: TrackingStatus;
  onTogglePause: () => void;
  onOpenDay: (iso: string) => void;
  onReview: (range: { start: string; days: number }) => void;
  onNavigate: (view: "timesheet" | "projects" | "categories") => void;
};

/**
 * Bugün: günün süresi ve hedefi, şu an takip edilen iş, bekleyen işler (atanmamış süre,
 * aktarılmamış günler, öneriler, bütçe) ve günün projeleri tek ekranda.
 */
export default function Today({ tracking, onTogglePause, onOpenDay, onReview, onNavigate }: Props) {
  const [data, setData] = useState<Data | null>(null);
  const [error, setError] = useState<string | null>(null);
  const todayIso = isoDate(today());

  const load = useCallback(async () => {
    const day = isoDate(today());
    const now = new Date();
    const yesterday = addDays(parseIsoDate(day), -1);
    const until = new Date(+yesterday + (+now - +parseIsoDate(day))).toISOString();
    const soft = <T,>(p: Promise<T>) => p.catch(() => null);
    try {
      const [report, prev, week, unassigned, pending, suggestions, budgets, goals] = await Promise.all([
        // Şerit için çalışma blokları gerekir (zaman çizelgesiyle gelir).
        api.report(day, 1, true),
        soft(api.report(isoDate(yesterday), 1, false, until)),
        soft(api.report(isoDate(startOfWeek(today())), 7, false)),
        soft(api.unassigned(day, 1)),
        soft(api.pendingTimesheetDays()),
        soft(api.suggestions()),
        soft(api.budgets()),
        soft(api.goals()),
      ]);
      setData({
        report,
        yesterday: prev,
        week,
        unassigned,
        pending: pending ?? [],
        suggestions,
        budgets,
        dailyHours: goals?.dailyHours ?? 8,
        projectGoals: goals?.projectGoals ?? [],
      });
      setError(null);
    } catch (e) {
      setError(friendlyError(e));
    }
  }, []);

  useEffect(() => {
    load();
    const id = setInterval(load, REFRESH_MS);
    return () => clearInterval(id);
  }, [load]);
  useChanged(load);
  useTauriEvent(api.onSync, load);

  return (
    <>
      <Toolbar title="Bugün">
        <span className="text-xs text-muted-foreground capitalize">{longDate.format(today())}</span>
        <Button variant="outline" size="sm" onClick={() => onOpenDay(todayIso)} title="Günü takvimde aç">
          <CalendarDays /> Takvimde aç
        </Button>
      </Toolbar>
      <div className="@container flex-1 overflow-y-auto px-5 pb-6">
        {error && <p className="pb-3 text-xs text-destructive selectable">{error}</p>}
        {!data && !error && (
          <div className="mx-auto grid max-w-[1100px] gap-4" aria-busy>
            <div className="skeleton h-28 rounded-xl" />
            <div className="skeleton h-40 rounded-xl" />
            <div className="skeleton h-24 rounded-xl" />
          </div>
        )}
        {data && (
          <div className="mx-auto grid max-w-[1100px] gap-4">
            <div className="grid gap-4 @[760px]:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
              <Hero data={data} tracking={tracking} />
              <Now tracking={tracking} onToggle={onTogglePause} />
            </div>
            <Queue data={data} todayIso={todayIso} onReview={onReview} onNavigate={onNavigate} />
            <Ribbon report={data.report} day={todayIso} onOpen={() => onOpenDay(todayIso)} />
            <Projects data={data} />
          </div>
        )}
      </div>
    </>
  );
}

function Label({ children }: { children: ReactNode }) {
  return <div className="text-[11px] font-medium text-muted-foreground">{children}</div>;
}

/** Günün süresi, düne göre farkı ve hedef halkası; takip sürüyorsa hedefe tahmini varış. */
function Hero({ data, tracking }: { data: Data; tracking: TrackingStatus }) {
  const total = data.report.totalSeconds;
  const target = data.dailyHours * 3600;
  const ratio = target ? total / target : 0;
  const before = data.yesterday?.totalSeconds;
  const diff = before !== undefined && before > 0 && total > 0 ? total - before : null;
  const left = target - total;
  const live = !tracking.paused && !tracking.needsPermission && !!tracking.current;
  return (
    <Card className="hero-surface gap-3">
      <CardContent className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <Label>Çalışma süresi</Label>
          <div className="mt-1 text-[28px] leading-none font-semibold tracking-tight tabular">
            {formatDuration(total)}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-1.5 text-[11px] text-muted-foreground tabular">
            {diff !== null && (
              <span>
                Dünün bu saatine göre{" "}
                <span className={diff >= 0 ? "text-success" : "text-destructive"}>
                  {diff >= 0 ? "+" : "−"}
                  {formatDuration(Math.abs(diff))}
                </span>
              </span>
            )}
            {target > 0 &&
              (left <= 0 ? (
                <span className="text-success">Günlük hedef doldu</span>
              ) : (
                <span>
                  {diff !== null && "· "}hedefe {formatDuration(left)}
                  {live && ` · tahmini ${formatTime(new Date(Date.now() + left * 1000))}`}
                </span>
              ))}
          </div>
        </div>
        {target > 0 && <GoalRing ratio={ratio} target={target} />}
      </CardContent>
    </Card>
  );
}

/** Şu an takip edilen pencere, projesi ve duraklat/devam et. */
function Now({ tracking, onToggle }: { tracking: TrackingStatus; onToggle: () => void }) {
  const cur = tracking.paused || tracking.needsPermission ? null : tracking.current;
  const project = cur?.project ?? null;
  return (
    <Card className="gap-2">
      <CardContent className="flex h-full flex-col gap-1.5">
        <Label>Şu an</Label>
        {tracking.needsPermission ? (
          <p className="text-[13px] font-medium text-amber-700 dark:text-amber-400">Takip için izin gerekli</p>
        ) : tracking.paused ? (
          <p className="text-[13px] font-medium">
            Duraklatıldı
            {tracking.pausedUntil && (
              <span className="font-normal text-muted-foreground">
                {" "}
                · bitiş {formatTime(new Date(tracking.pausedUntil))}
              </span>
            )}
          </p>
        ) : cur ? (
          <>
            <div className="flex min-w-0 items-center gap-2 text-[13px] font-semibold">
              {project ? (
                <>
                  <i className="size-2.5 shrink-0 rounded-full" style={{ background: `var(--c${project.color})` }} />
                  <span className="truncate">{project.name}</span>
                  <span className="ml-auto shrink-0 text-xs font-normal text-muted-foreground tabular">
                    bugün {formatDuration(project.secondsToday)}
                  </span>
                </>
              ) : (
                <>
                  <i
                    className="size-2.5 shrink-0 rounded-full border border-muted-foreground/40"
                    style={{ background: HATCH }}
                  />
                  <span className="truncate text-muted-foreground">Projesiz</span>
                </>
              )}
            </div>
            <p className="truncate text-xs text-muted-foreground" title={cur.title || undefined}>
              {cur.appName}
              {cur.title && ` · ${cur.title}`}
            </p>
          </>
        ) : (
          <p className="text-[13px] text-muted-foreground">Boşta</p>
        )}
        {!tracking.needsPermission && (
          <div className="mt-auto pt-1">
            <Button variant="outline" size="sm" onClick={onToggle}>
              {tracking.paused ? <Play /> : <Pause />}
              {tracking.paused ? "Takibe devam et" : "Duraklat"}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

type Task = {
  key: string;
  icon: ReactNode;
  tone: "brand" | "primary" | "warn";
  title: string;
  detail?: string;
  action: string;
  run: () => void;
};

/** Sırada: bekleyen işler, her biri tek tıkla ilgili sayfada açılır. */
function Queue({
  data,
  todayIso,
  onReview,
  onNavigate,
}: {
  data: Data;
  todayIso: string;
  onReview: Props["onReview"];
  onNavigate: Props["onNavigate"];
}) {
  const tags = useMemo(() => tagMap(data.week?.tags ?? data.report.tags), [data]);
  const tasks: Task[] = [];
  const u = data.unassigned;
  // Proje kullanılmıyorsa her süre atanmamıştır; uyarı gürültü olur.
  const hasProjects = data.report.tags.some((t) => t.kind === "project");
  if (u && hasProjects && u.totalSeconds + u.idleSeconds >= UNASSIGNED_MIN) {
    const top = u.groups[0];
    const likely = top?.likelyProject ? tags.get(top.likelyProject)?.name : undefined;
    tasks.push({
      key: "unassigned",
      icon: <Inbox />,
      tone: "brand",
      title: `${formatDuration(u.totalSeconds)} projeye atanmamış${u.idleSeconds > 0 ? `, ${formatDuration(u.idleSeconds)} boşta` : ""}`,
      detail: top ? `En çok: ${top.label}${likely ? ` (önerilen: ${likely})` : ""}` : undefined,
      action: "Gözden geçir",
      run: () => onReview({ start: todayIso, days: 1 }),
    });
  }
  // Bugün sürüyor: yalnızca önceki günler bekleyen sayılır.
  const pending = data.pending.filter((d) => d < todayIso);
  if (pending.length > 0)
    tasks.push({
      key: "pending",
      icon: <FileSpreadsheet />,
      tone: "primary",
      title:
        pending.length === 1
          ? `${capitalize(dayName.format(parseIsoDate(pending[0])))} zaman çizelgesine aktarılmadı`
          : `${pending.length} gün zaman çizelgesine aktarılmadı`,
      detail: pending.length > 1 ? pending.map((d) => dayName.format(parseIsoDate(d))).join(", ") : undefined,
      action: "Çizelgeyi aç",
      run: () => onNavigate("timesheet"),
    });
  for (const b of data.budgets?.projects ?? []) {
    const state = budgetState(b);
    if (state === "ok") continue;
    const name = tags.get(b.id)?.name;
    if (!name) continue;
    const dayHours = data.budgets?.dayHours ?? 8;
    tasks.push({
      key: `budget-${b.id}`,
      icon: <Wallet />,
      tone: "warn",
      title: `${name} bütçesi ${state === "over" ? "aşıldı" : `%${Math.round(budgetRatio(b) * 100)} doldu`}`,
      detail: `${formatDays(b.usedSeconds, dayHours)} / ${formatDays(b.budgetSeconds, dayHours)} adam-gün`,
      action: "Projeler",
      run: () => onNavigate("projects"),
    });
  }
  const s = data.suggestions;
  if (s && s.projects.length > 0)
    tasks.push({
      key: "projects",
      icon: <FolderKanban />,
      tone: "primary",
      title: `${s.projects.length} proje önerisi`,
      detail: s.projects
        .slice(0, 3)
        .map((p) => p.name)
        .join(", "),
      action: "Bak",
      run: () => onNavigate("projects"),
    });
  if (s && s.categories.length > 0)
    tasks.push({
      key: "categories",
      icon: <Tags />,
      tone: "primary",
      title: `${s.categories.length} kategori önerisi`,
      action: "Bak",
      run: () => onNavigate("categories"),
    });

  return (
    <Card className="gap-1 py-3">
      <CardContent>
        <div className="flex items-center gap-1.5 pb-1">
          <Label>Sırada</Label>
          {tasks.length > 0 && <span className="text-[11px] text-muted-foreground tabular">· {tasks.length}</span>}
        </div>
        {tasks.length === 0 ? (
          <p className="py-1.5 text-xs text-muted-foreground">
            Bekleyen iş yok: süre projelere atanmış, günler aktarılmış.
          </p>
        ) : (
          <ul className="divide-y">
            {tasks.map((t) => (
              <li key={t.key}>
                <button
                  onClick={t.run}
                  className="group flex w-full items-center gap-3 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
                >
                  <span
                    className={cn(
                      "grid size-7 shrink-0 place-items-center rounded-lg [&_svg]:size-3.5",
                      t.tone === "brand"
                        ? "bg-brand text-white shadow-sm shadow-brand-2/30"
                        : t.tone === "warn"
                          ? "bg-amber-500/15 text-amber-700 dark:text-amber-400"
                          : "bg-primary/10 text-primary",
                    )}
                  >
                    {t.icon}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-xs font-semibold">{t.title}</span>
                    {t.detail && <span className="block truncate text-[11px] text-muted-foreground">{t.detail}</span>}
                  </span>
                  <span className="flex shrink-0 items-center gap-0.5 text-xs text-muted-foreground group-hover:text-foreground">
                    {t.action}
                    <ChevronRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

/** Günün çalışma blokları yatay şeritte, proje renginde; atanmamış bloklar taralı. */
function Ribbon({ report, day: iso, onOpen }: { report: Report; day: string; onOpen: () => void }) {
  const tags = useMemo(() => tagMap(report.tags), [report]);
  const blocks = report.work.blocks;
  const day = +parseIsoDate(iso);
  const hour = 3600_000;
  const now = Date.now();
  const starts = blocks.map((b) => +new Date(b.start));
  const ends = blocks.map((b) => +new Date(b.end));
  // En az 08–18; erken başlayan, geç biten ya da hâlâ süren gün şeridi genişletir.
  const nowIn = now > day && now < day + 24 * hour ? [now] : [];
  const from = Math.min(day + 8 * hour, ...starts.map((t) => Math.floor((t - day) / hour) * hour + day));
  const to = Math.min(
    day + 24 * hour,
    Math.max(day + 18 * hour, ...[...ends, ...nowIn].map((t) => Math.ceil((t - day) / hour) * hour + day)),
  );
  const span = to - from;
  const pct = (t: number) => `${((t - from) / span) * 100}%`;
  const hours: number[] = [];
  const stepH = span / hour > 12 ? 3 : 2;
  for (let t = from; t <= to; t += stepH * hour) hours.push(t);
  const showNow = now > from && now < to && now < day + 24 * hour;
  return (
    <Card className="gap-2 py-3">
      <CardContent>
        <div className="flex items-center justify-between pb-2">
          <Label>Gün</Label>
          <button className="text-[11px] text-muted-foreground hover:text-foreground" onClick={onOpen}>
            Takvimde aç →
          </button>
        </div>
        <button
          className="relative block h-6 w-full overflow-hidden rounded-md bg-muted outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          onClick={onOpen}
          aria-label="Günü takvimde aç"
        >
          {blocks.map((b, i) => {
            const tag = b.projectId ? tags.get(b.projectId) : undefined;
            return (
              <i
                key={i}
                className="absolute inset-y-0 border-r border-card"
                title={`${formatTime(new Date(b.start))}–${formatTime(new Date(b.end))} · ${tag?.name ?? UNASSIGNED}`}
                style={{
                  left: pct(starts[i]),
                  width: `${((ends[i] - starts[i]) / span) * 100}%`,
                  background: tag ? tagColor(tag) : HATCH,
                }}
              />
            );
          })}
          {showNow && (
            <span className="absolute -inset-y-0.5 w-0.5 rounded-full bg-foreground" style={{ left: pct(now) }} />
          )}
        </button>
        <div className="relative mt-1 h-3 text-[10px] text-muted-foreground tabular">
          {hours.map((t, i) => (
            <span
              key={t}
              className={cn(
                "absolute",
                i === 0 ? "" : i === hours.length - 1 ? "-translate-x-full" : "-translate-x-1/2",
              )}
              style={{ left: pct(t) }}
            >
              {String(new Date(t).getHours()).padStart(2, "0")}
            </span>
          ))}
        </div>
        {blocks.length === 0 && (
          <p className="pt-2 text-xs text-muted-foreground">
            Henüz kayıt yok; Kum çalışırken şerit kendiliğinden dolar.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** Günün projeleri: bugünkü süre, haftalık toplam ve varsa haftalık hedefe ilerleme. */
function Projects({ data }: { data: Data }) {
  const tags = useMemo(() => tagMap(data.week?.tags ?? data.report.tags), [data]);
  const weekOf = new Map((data.week?.projects ?? []).map((b) => [b.id, b.seconds]));
  const rows = data.report.projects
    .filter((b) => b.id !== null && tags.has(b.id))
    .map((b) => {
      const goal = data.projectGoals.find((g) => g.projectId === b.id);
      return { id: b.id!, today: b.seconds, week: weekOf.get(b.id) ?? b.seconds, goal: goal ? goal.minutes * 60 : 0 };
    });
  if (rows.length === 0) return null;
  return (
    <Card className="gap-3 py-3">
      <CardContent>
        <div className="pb-2.5">
          <Label>Projeler</Label>
        </div>
        <ul className="grid gap-x-6 gap-y-3 @[640px]:grid-cols-2 @[960px]:grid-cols-3">
          {rows.map((r) => {
            const tag = tags.get(r.id);
            const color = tagColor(tag);
            const done = r.goal > 0 && r.week >= r.goal;
            return (
              <li key={r.id} className="min-w-0 space-y-1.5 text-xs">
                <div className="flex items-center gap-2">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: color }} />
                  <span className="min-w-0 flex-1 truncate font-medium">{tag?.name}</span>
                  <span className="text-muted-foreground tabular">{formatDuration(r.today)}</span>
                </div>
                {r.goal > 0 && (
                  <div className="h-1 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full"
                      style={{
                        width: `${Math.min(100, (r.week / r.goal) * 100)}%`,
                        background: done ? "var(--success)" : color,
                      }}
                    />
                  </div>
                )}
                <div className="text-[11px] text-muted-foreground tabular">
                  Bu hafta {formatDuration(r.week)}
                  {r.goal > 0 && ` / ${formatDuration(r.goal)}`}
                </div>
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr") + s.slice(1);
}
