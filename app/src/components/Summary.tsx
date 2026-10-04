import { useEffect, useState } from "react";
import { Check, ChevronRight, Inbox, TrendingDown, TrendingUp } from "lucide-react";
import type { CategoryLimit, ProjectGoal, Report, Tag, Taxonomy } from "../api";
import { api, formatDuration } from "../api";
import { clientColor, NO_CLIENT, NO_PROJECT, UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import type { Mode } from "./ReportView";
import { Card, CardContent } from "./ui/card";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

type Props = {
  report: Report;
  previous: Report | null;
  tags: Map<string, Tag>;
  days: number;
  title: string;
  mode: Mode;
  /** Ayarlardaki günlük hedef (saat). */
  dailyHours: number;
  /** Gösterilecek kategori limitleri (yalnızca gün görünümünde). */
  limits: CategoryLimit[];
  /** Hafta görünümünde proje hedefleri. */
  projectGoals: ProjectGoal[];
  /** Projeye atanmamış süreyi gözden geçir. */
  onReview: () => void;
};

/** Sağ panel: süre, hedef, kırılım ve mola metrikleri. */
export default function Summary({
  report,
  previous,
  tags,
  days,
  mode,
  title,
  dailyHours,
  limits,
  projectGoals,
  onReview,
}: Props) {
  const f = report.work;
  const target = dailyHours * 3600 * activeDays(report, days);
  const ratio = target ? report.totalSeconds / target : 0;
  const breakSecs = f.breakSeconds;

  return (
    <aside className="min-w-0 space-y-3">
      <div className="px-1 pt-0.5 text-xs font-semibold text-muted-foreground">Özet · {title}</div>

      <Card className="hero-surface gap-3">
        <CardContent>
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <Label>Çalışma süresi</Label>
              <div className="mt-1 text-[28px] leading-none font-semibold tracking-tight tabular">
                {formatDuration(report.totalSeconds)}
              </div>
              <Delta now={report.totalSeconds} before={previous?.totalSeconds} unit={DELTA_UNIT[mode]} />
            </div>
            <GoalRing ratio={ratio} target={target} />
          </div>
        </CardContent>
      </Card>

      <UnassignedCard
        seconds={report.projects.find((b) => b.id === null)?.seconds ?? 0}
        idle={report.idleSeconds}
        hasProjects={report.tags.some((t) => t.kind === "project")}
        onReview={onReview}
      />

      <BreakdownCard report={report} tags={tags} />

      <LimitsCard report={report} tags={tags} limits={limits} />
      <ProjectGoalsCard report={report} tags={tags} goals={projectGoals} />

      {mode !== "day" && <Highlights report={report} dailyHours={dailyHours} />}

      <Card className="gap-3">
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between">
            <Label>Verimlilik</Label>
            <span className="text-[11px] text-muted-foreground tabular">
              Toplam {formatDuration(report.totalSeconds + breakSecs)}
            </span>
          </div>
          <Metrics
            parts={[
              {
                label: "Çalışma",
                secs: report.totalSeconds,
                color: "color-mix(in srgb, var(--primary) 45%, transparent)",
              },
              { label: "Mola", secs: breakSecs, color: "color-mix(in srgb, var(--muted-foreground) 35%, transparent)" },
            ]}
          />
          <div className="text-[11px] text-muted-foreground">
            {(f.switchesPerHourX10 / 10).toLocaleString("tr-TR")} uygulama geçişi/sa
          </div>
          {report.idleSeconds > 0 && (
            <button
              className="text-left text-[11px] text-muted-foreground hover:text-foreground"
              title="Projeye atarsan çalışma süresine eklenir."
              onClick={onReview}
            >
              Bilgisayardan uzakta: {formatDuration(report.idleSeconds)} →
            </button>
          )}
        </CardContent>
      </Card>
    </aside>
  );
}

/** Hedefe ulaşma halkası: kum gradyanı, hedef dolunca yeşil. */
function GoalRing({ ratio, target }: { ratio: number; target: number }) {
  const r = 26;
  const c = 2 * Math.PI * r;
  const done = ratio >= 1;
  return (
    <div className="relative grid size-[68px] shrink-0 place-items-center" title={`Hedef: ${formatDuration(target)}`}>
      <svg viewBox="0 0 68 68" className="absolute inset-0 -rotate-90" aria-hidden>
        <defs>
          <linearGradient id="goal-grad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--brand-1)" />
            <stop offset="100%" stopColor="var(--brand-2)" />
          </linearGradient>
        </defs>
        <circle cx="34" cy="34" r={r} fill="none" stroke="var(--muted)" strokeWidth="6" />
        <circle
          cx="34"
          cy="34"
          r={r}
          fill="none"
          stroke={done ? "var(--success)" : "url(#goal-grad)"}
          strokeWidth="6"
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - Math.min(1, ratio))}
          className="transition-[stroke-dashoffset] duration-700 ease-out"
        />
      </svg>
      <div className="text-center leading-none">
        <div className="text-[13px] font-semibold tabular">%{Math.round(ratio * 100)}</div>
        <div className="mt-0.5 text-[9px] text-muted-foreground">hedef</div>
      </div>
    </div>
  );
}

/** Projeye düşmeyen süre varsa: ne kadar olduğu ve gözden geçirme kısayolu. */
function UnassignedCard({
  seconds,
  idle,
  hasProjects,
  onReview,
}: {
  seconds: number;
  idle: number;
  hasProjects: boolean;
  onReview: () => void;
}) {
  // Proje kullanılmıyorsa her süre projesizdir; uyarı gürültü olur.
  if (!hasProjects || seconds + idle < 5 * 60) return null;
  return (
    <button
      onClick={onReview}
      className="group flex w-full items-center gap-3 rounded-xl border border-brand-2/25 bg-brand-soft px-3.5 py-2.5 text-left transition-colors hover:border-brand-2/50"
    >
      <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-brand text-white shadow-sm shadow-brand-2/30">
        <Inbox className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-semibold">{formatDuration(seconds)} projeye atanmamış</span>
        <span className="block text-[11px] text-muted-foreground">
          {idle > 0 ? `ve ${formatDuration(idle)} boşta · ` : ""}Gözden geçir
        </span>
      </span>
      <ChevronRight className="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
    </button>
  );
}

const DELTA_UNIT: Record<Mode, string> = {
  day: "düne göre",
  week: "geçen haftaya göre",
  month: "geçen aya göre",
};

function Label({ children }: { children: React.ReactNode }) {
  return <div className="text-[11px] font-medium text-muted-foreground">{children}</div>;
}

function activeDays(report: Report, days: number): number {
  if (days === 1) return 1;
  return Math.max(1, report.days.filter((d) => d.seconds > 0).length);
}

function Delta({ now, before, unit }: { now: number; before?: number; unit: string }) {
  // Dönemde henüz kayıt yoksa "-%100" yanıltıcı olur; kıyas gösterilmez.
  if (before === undefined || before === 0 || now === 0)
    return <div className="mt-1.5 text-[11px] text-muted-foreground">—</div>;
  const pct = Math.round(((now - before) / before) * 100);
  const up = pct >= 0;
  const Icon = up ? TrendingUp : TrendingDown;
  return (
    <div
      className={cn(
        "mt-1.5 flex flex-wrap items-center gap-x-1 text-[11px] font-medium tabular",
        up ? "text-success" : "text-destructive",
      )}
    >
      <span className="flex items-center gap-1 whitespace-nowrap">
        <Icon className="size-3.5" />
        {`${up ? "+" : "-"}%${Math.abs(pct)}`}
      </span>
      <span className="font-normal whitespace-nowrap text-muted-foreground">{unit}</span>
    </div>
  );
}

type Tab = "categories" | "projects" | "clients" | "apps";

function BreakdownCard({ report, tags }: { report: Report; tags: Map<string, Tag> }) {
  const [tab, setTab] = useState<Tab>("categories");
  // Müşteriler raporda yok; projelerin müşterisi buradan alınır, süre müşteriye göre toplanır.
  const [taxonomy, setTaxonomy] = useState<Taxonomy | null>(null);
  useEffect(() => {
    if (tab === "clients") api.taxonomy().then(setTaxonomy, () => {});
  }, [tab, report]);
  const items =
    tab === "clients"
      ? byClient(report, taxonomy)
      : tab === "apps"
        ? report.apps.map((a) => ({
            key: a.appId,
            name: a.appName,
            secs: a.seconds,
            color: tagColor(a.categoryId ? tags.get(a.categoryId) : undefined),
          }))
        : (tab === "categories" ? report.categories : report.projects).map((b) => {
            const tag = b.id ? tags.get(b.id) : undefined;
            return {
              key: b.id ?? "none",
              name: tag?.name ?? (tab === "categories" ? UNCATEGORIZED : NO_PROJECT),
              secs: b.seconds,
              color: tagColor(tag),
            };
          });
  const top = items.slice(0, 5);
  const rest = items.slice(5).reduce((s, i) => s + i.secs, 0);
  const donut = rest > 0 ? [...top, { key: "rest", name: "Diğer", secs: rest, color: "var(--c0)" }] : top;

  return (
    <Card className="gap-3">
      <CardContent className="space-y-3">
        <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)}>
          <TabsList className="w-full">
            {(
              [
                ["categories", "Kategori"],
                ["projects", "Proje"],
                ["clients", "Müşteri"],
                ["apps", "Uygulama"],
              ] as const
            ).map(([v, label]) => (
              <TabsTrigger key={v} value={v} className="min-w-0 px-1 text-[11px]">
                <span className="truncate">{label}</span>
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        {tab === "projects" && items.length > 0 && items.every((i) => i.key === "none") ? (
          <p className="py-2 text-xs text-muted-foreground">
            Bu dönemde bir projeye düşen süre yok. Proje eklemek için kenar çubuğunda Projeler.
          </p>
        ) : items.length === 0 ? (
          <p className="py-2 text-xs text-muted-foreground">Kayıt yok.</p>
        ) : (
          <div className="flex items-center gap-4">
            <Donut parts={donut.map((d) => ({ value: d.secs, color: d.color }))} />
            <ul className="min-w-0 flex-1 space-y-1.5">
              {donut.map((d) => (
                <li key={d.key} className="flex items-center gap-2 text-xs">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: d.color }} />
                  <span className="min-w-0 flex-1 truncate">{d.name}</span>
                  <span className="text-muted-foreground tabular">{formatDuration(d.secs)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/** Projelerin süresi müşteriye göre; müşterisi olmayan proje ve projesiz süre "Müşterisiz". */
function byClient(report: Report, taxonomy: Taxonomy | null) {
  if (!taxonomy) return [];
  const totals = new Map<string, number>();
  for (const b of report.projects) {
    const client = (b.id && taxonomy.projectClients[b.id]) || "none";
    totals.set(client, (totals.get(client) ?? 0) + b.seconds);
  }
  return [...totals.entries()]
    .map(([key, secs]) => ({
      key,
      name: taxonomy.clients.find((c) => c.id === key)?.name ?? NO_CLIENT,
      secs,
      color: clientColor(taxonomy.clients, key === "none" ? null : key),
    }))
    .sort((a, b) => (a.key === "none" ? 1 : b.key === "none" ? -1 : b.secs - a.secs));
}

/** Halka grafik; dilimler arasında küçük boşluk. */
function Donut({ parts, size = 84 }: { parts: { value: number; color: string }[]; size?: number }) {
  const total = parts.reduce((s, p) => s + p.value, 0) || 1;
  const stroke = 10;
  const r = size / 2 - stroke / 2 - 1;
  const c = 2 * Math.PI * r;
  const gap = parts.length > 1 ? 2.5 : 0;
  let offset = 0;
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="shrink-0" aria-hidden="true">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--muted)" strokeWidth={stroke} />
      {parts.map((p, i) => {
        const len = Math.max(0, (p.value / total) * c - gap);
        const el = (
          <circle
            key={i}
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={p.color}
            strokeWidth={stroke}
            strokeDasharray={`${len} ${c - len}`}
            strokeDashoffset={-offset}
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        );
        offset += (p.value / total) * c;
        return el;
      })}
    </svg>
  );
}

function Metrics({ parts }: { parts: { label: string; secs: number; color: string }[] }) {
  return (
    <>
      <div className="flex h-2 gap-0.5 overflow-hidden rounded-full bg-muted">
        {parts
          .filter((p) => p.secs > 0)
          .map((p) => (
            <span
              key={p.label}
              className="h-full first:rounded-l-full last:rounded-r-full"
              style={{ flexGrow: p.secs, background: p.color }}
            />
          ))}
      </div>
      <ul className="flex justify-between gap-2">
        {parts.map((p) => (
          <li key={p.label} className="whitespace-nowrap">
            <div className="flex items-center gap-1.5 text-[11px]">
              <i className="size-2 shrink-0 rounded-full" style={{ background: p.color }} />
              <span>{p.label}</span>
            </div>
            <div className="mt-0.5 pl-3.5 text-[11px] text-muted-foreground tabular">{formatDuration(p.secs)}</div>
          </li>
        ))}
      </ul>
    </>
  );
}

/** Proje hedefleri: bu haftaki süre / hedef; dolunca onay işareti. */
function ProjectGoalsCard({ report, tags, goals }: { report: Report; tags: Map<string, Tag>; goals: ProjectGoal[] }) {
  const rows = goals.flatMap((g) => {
    const tag = tags.get(g.projectId);
    if (!tag) return [];
    const used = report.projects.find((p) => p.id === g.projectId)?.seconds ?? 0;
    return [{ tag, used, target: g.minutes * 60 }];
  });
  if (rows.length === 0) return null;
  return (
    <Card className="gap-3">
      <CardContent className="space-y-3">
        <Label>Proje hedefleri</Label>
        <ul className="space-y-2.5">
          {rows.map(({ tag, used, target }) => {
            const ratio = used / target;
            return (
              <li key={tag.id} className="space-y-1">
                <div className="flex items-center gap-2 text-xs">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
                  <span className="min-w-0 flex-1 truncate">{tag.name}</span>
                  {ratio >= 1 && <Check className="size-3.5 text-success" aria-label="Hedef doldu" />}
                  <span className={cn("tabular", ratio >= 1 ? "font-medium" : "text-muted-foreground")}>
                    {formatDuration(used)} / {formatDuration(target)}
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full"
                    style={{ width: `${Math.min(100, ratio * 100)}%`, background: tagColor(tag) }}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}

/** Kategori limitleri: kullanılan / sınır, dolunca kırmızı. */
function LimitsCard({ report, tags, limits }: { report: Report; tags: Map<string, Tag>; limits: CategoryLimit[] }) {
  const rows = limits.flatMap((l) => {
    const tag = tags.get(l.categoryId);
    if (!tag) return [];
    const used = report.categories.find((c) => c.id === l.categoryId)?.seconds ?? 0;
    return [{ tag, used, limit: l.minutes * 60 }];
  });
  if (rows.length === 0) return null;
  return (
    <Card className="gap-3">
      <CardContent className="space-y-3">
        <Label>Limitler</Label>
        <ul className="space-y-2.5">
          {rows.map(({ tag, used, limit }) => {
            const ratio = used / limit;
            return (
              <li key={tag.id} className="space-y-1">
                <div className="flex items-center gap-2 text-xs">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
                  <span className="min-w-0 flex-1 truncate">{tag.name}</span>
                  <span
                    className={cn("tabular", ratio >= 1 ? "font-medium text-destructive" : "text-muted-foreground")}
                  >
                    {formatDuration(used)} / {formatDuration(limit)}
                  </span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full"
                    style={{
                      width: `${Math.min(100, ratio * 100)}%`,
                      background: ratio >= 1 ? "var(--destructive)" : tagColor(tag),
                    }}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}

const weekdayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "short" });

/** Hafta/ay için öne çıkanlar: en yoğun gün, ortalama, hedef günleri, en çok kullanılan uygulama. */
function Highlights({ report, dailyHours }: { report: Report; dailyHours: number }) {
  const active = report.days.filter((d) => d.seconds > 0);
  if (active.length < 2) return null;
  const busiest = active.reduce((a, b) => (b.seconds > a.seconds ? b : a));
  const goalDays = active.filter((d) => d.seconds >= dailyHours * 3600).length;
  const average = Math.round(active.reduce((s, d) => s + d.seconds, 0) / active.length);
  const topApp = report.apps[0];
  const day = (iso: string) => weekdayFmt.format(new Date(iso));
  const rows: [string, string, string?][] = [
    ["En yoğun gün", formatDuration(busiest.seconds), day(busiest.start)],
    ["Günlük ortalama", formatDuration(average), `${active.length} aktif gün`],
    ["Hedef tutan gün", `${goalDays} / ${active.length}`, `günde ${dailyHours} sa`],
  ];
  if (topApp) rows.push(["En çok kullanılan", topApp.appName, formatDuration(topApp.seconds)]);
  return (
    <Card className="gap-3">
      <CardContent className="space-y-2.5">
        <Label>Öne çıkanlar</Label>
        <ul className="space-y-2">
          {rows.map(([label, value, hint]) => (
            <li key={label} className="flex items-baseline justify-between gap-3 text-xs">
              <span className="shrink-0 whitespace-nowrap text-muted-foreground">{label}</span>
              <span className="min-w-0 truncate text-right">
                <span className="font-medium tabular">{value}</span>
                {hint && <span className="text-muted-foreground"> · {hint}</span>}
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}
