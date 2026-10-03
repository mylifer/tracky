import { useState } from "react";
import { Check, TrendingDown, TrendingUp } from "lucide-react";
import type { CategoryLimit, ProjectGoal, Report, Tag } from "../api";
import { formatDuration } from "../api";
import { NO_PROJECT, UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import type { Mode } from "./ReportView";
import { ScoreRing, scoreLabel } from "./Stats";
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
};

/** Sağ panel: süre, hedef, kırılım ve odak metrikleri. */
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
}: Props) {
  const f = report.focus;
  const target = dailyHours * 3600 * activeDays(report, days);
  const ratio = target ? report.totalSeconds / target : 0;
  const breakSecs = f.breakSeconds;
  const otherWork = Math.max(0, report.totalSeconds - f.focusSeconds);

  return (
    <aside className="min-w-0 space-y-3">
      <div className="px-1 pt-0.5 text-xs font-semibold text-muted-foreground">Özet · {title}</div>

      <Card className="gap-3">
        <CardContent className="space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div>
              <Label>Çalışma süresi</Label>
              <div className="mt-1 text-[26px] leading-none font-semibold tracking-tight tabular">
                {formatDuration(report.totalSeconds)}
              </div>
              <Delta now={report.totalSeconds} before={previous?.totalSeconds} unit={DELTA_UNIT[mode]} />
            </div>
            <div className="text-right">
              <Label>Hedef</Label>
              <div className="mt-1 text-[15px] font-semibold tabular">%{Math.round(ratio * 100)}</div>
              <div className="text-[11px] text-muted-foreground tabular">{formatDuration(target)}</div>
            </div>
          </div>
          <div className="h-1.5 overflow-hidden rounded-full bg-muted">
            <div
              className={cn("h-full rounded-full transition-[width]", ratio >= 1 ? "bg-success" : "bg-primary")}
              style={{ width: `${Math.min(100, ratio * 100)}%` }}
            />
          </div>
        </CardContent>
      </Card>

      <BreakdownCard report={report} tags={tags} />

      <LimitsCard report={report} tags={tags} limits={limits} />
      <ProjectGoalsCard report={report} tags={tags} goals={projectGoals} />

      {mode !== "day" && <Highlights report={report} dailyHours={dailyHours} />}

      <div className="grid grid-cols-2 gap-3">
        <Card className="gap-2 py-3.5">
          <CardContent className="px-3.5">
            <Label>Odak skoru</Label>
            <div className="mt-2 flex items-center gap-2.5">
              <ScoreRing score={f.score} size={46} />
              <span className="text-xs leading-tight font-medium">{scoreLabel(f.score)}</span>
            </div>
          </CardContent>
        </Card>
        <Card className="gap-2 py-3.5">
          <CardContent className="px-3.5">
            <Label>Odak süresi</Label>
            <div className="mt-1.5 text-[17px] font-semibold tabular">{formatDuration(f.focusSeconds)}</div>
            <Delta now={f.focusSeconds} before={previous?.focus.focusSeconds} unit={DELTA_SHORT[mode]} />
          </CardContent>
        </Card>
      </div>

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
              { label: "Odak", secs: f.focusSeconds, color: "var(--focus)" },
              { label: "Diğer çalışma", secs: otherWork, color: "color-mix(in srgb, var(--primary) 45%, transparent)" },
              { label: "Mola", secs: breakSecs, color: "color-mix(in srgb, var(--muted-foreground) 35%, transparent)" },
            ]}
          />
          <div className="text-[11px] text-muted-foreground">
            {(f.switchesPerHourX10 / 10).toLocaleString("tr-TR")} geçiş/sa · en uzun odak{" "}
            {formatDuration(f.longestFocusSeconds)}
          </div>
        </CardContent>
      </Card>
    </aside>
  );
}

const DELTA_UNIT: Record<Mode, string> = {
  day: "düne göre",
  week: "geçen haftaya göre",
  month: "geçen aya göre",
};
const DELTA_SHORT: Record<Mode, string> = { day: "dün", week: "geçen hafta", month: "geçen ay" };

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

type Tab = "categories" | "projects" | "apps";

function BreakdownCard({ report, tags }: { report: Report; tags: Map<string, Tag> }) {
  const [tab, setTab] = useState<Tab>("categories");
  const items =
    tab === "apps"
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
            <TabsTrigger value="categories">Kategoriler</TabsTrigger>
            <TabsTrigger value="projects">Projeler</TabsTrigger>
            <TabsTrigger value="apps">Uygulamalar</TabsTrigger>
          </TabsList>
        </Tabs>
        {tab === "projects" && items.length > 0 && items.every((i) => i.key === "none") ? (
          <p className="py-2 text-xs text-muted-foreground">
            Bu dönemde bir projeye düşen süre yok. Proje eklemek için kenar çubuğunda Kategoriler ve projeler →
            Projeler.
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

/** Hafta/ay için öne çıkanlar: en yoğun ve en odaklı gün, ortalama, hedef günleri, en çok kullanılan uygulama. */
function Highlights({ report, dailyHours }: { report: Report; dailyHours: number }) {
  const active = report.days.filter((d) => d.seconds > 0);
  if (active.length < 2) return null;
  const busiest = active.reduce((a, b) => (b.seconds > a.seconds ? b : a));
  const focused = active.reduce((a, b) => (b.focusScore > a.focusScore ? b : a));
  const goalDays = active.filter((d) => d.seconds >= dailyHours * 3600).length;
  const average = Math.round(active.reduce((s, d) => s + d.seconds, 0) / active.length);
  const topApp = report.apps[0];
  const day = (iso: string) => weekdayFmt.format(new Date(iso));
  const rows: [string, string, string?][] = [
    ["En yoğun gün", formatDuration(busiest.seconds), day(busiest.start)],
    ["En odaklı gün", `skor ${focused.focusScore}`, day(focused.start)],
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
