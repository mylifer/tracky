import { useEffect, useState } from "react";
import { ChevronLeft } from "lucide-react";
import { api, formatDuration, type ProjectStats, type Tag, type UsageTotal } from "../../api";
import {
  BurnChart,
  ChartCard,
  Heatmap,
  HourBars,
  PeriodTabs,
  RankList,
  Stat,
  StackedBars,
  Versus,
  weekLabels,
} from "../../components/charts";
import { TagEditor, type Run } from "../../components/TagEditor";
import { ErrorText } from "../../components/settings";
import { Button } from "../../components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../../components/ui/tabs";
import { budgetRatio, formatDays, weeksLeft } from "../../lib/budget";
import { addDays, formatDate, isoDate, parseIsoDate, startOfWeek, today } from "../../lib/dates";
import { friendlyError, useChanged } from "../../lib/feedback";
import { formatHours, PERIOD_WORD, WEEKS, type Insights, type Period } from "../../lib/insights";
import { NO_CLIENT, tagColor, UNCATEGORIZED } from "../../lib/tags";

export type DetailTab = "overview" | "settings";

/** Proje profilinin kapsadığı gün sayısı (ısı haritası: 26 tam hafta, bu hafta dahil). */
const DAYS = WEEKS * 7;

/**
 * Proje detayı: dönem özeti, gün gün ısı haritası, haftalık süre, günün saatleri, uygulamalar,
 * kategoriler, en çok açılan pencereler ve bütçe tahmini. "Kurallar ve ayarlar" sekmesinde
 * eski Projeler sayfasındaki düzenleyici.
 */
export default function ProjectDetail({
  project,
  data,
  period,
  onPeriod,
  tab,
  onTab,
  apps,
  run,
  error,
  onBack,
}: {
  project: Tag;
  data: Insights;
  period: Period;
  onPeriod: (p: Period) => void;
  tab: DetailTab;
  onTab: (t: DetailTab) => void;
  apps: UsageTotal[];
  run: Run;
  error: string | null;
  onBack: () => void;
}) {
  const color = tagColor(project);
  const client = data.clients.find((c) => c.id === data.links[project.id]);
  return (
    <div className="mx-auto w-full max-w-6xl space-y-5 px-6 pt-2 pb-10">
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="ghost" size="sm" className="-ml-2 text-muted-foreground" onClick={onBack}>
          <ChevronLeft /> Projeler
        </Button>
        <h1 className="flex min-w-0 items-center gap-2.5 text-lg font-semibold tracking-tight">
          <i className="size-3 shrink-0 rounded-full" style={{ background: color }} />
          <span className="truncate">{project.name}</span>
          <span className="shrink-0 text-[13px] font-normal text-muted-foreground">
            {client?.name ?? NO_CLIENT}
            {project.archived && " · arşivde"}
          </span>
        </h1>
        <div className="flex-1" />
        {tab === "overview" && <PeriodTabs value={period} onChange={onPeriod} />}
      </div>
      <Tabs value={tab} onValueChange={(v) => onTab(v as DetailTab)}>
        <TabsList aria-label="Bölüm">
          <TabsTrigger value="overview" className="px-3">
            Genel bakış
          </TabsTrigger>
          <TabsTrigger value="settings" className="px-3">
            Kurallar ve ayarlar
          </TabsTrigger>
        </TabsList>
      </Tabs>
      <ErrorText>{error}</ErrorText>
      {tab === "overview" ? (
        <Overview project={project} data={data} period={period} color={color} />
      ) : (
        <div className="max-w-3xl rounded-xl border bg-card px-5 py-4 shadow-xs">
          <TagEditor
            tag={project}
            rules={data.rules.filter((r) => r.tagId === project.id)}
            apps={apps}
            clients={data.clients}
            clientId={data.links[project.id] ?? null}
            budget={data.budgets?.projects.find((b) => b.id === project.id)}
            dayHours={data.budgets?.dayHours ?? 8}
            run={run}
            allTags={data.tags}
          />
        </div>
      )}
    </div>
  );
}

function Overview({ project, data, period, color }: { project: Tag; data: Insights; period: Period; color: string }) {
  const [stats, setStats] = useState<ProjectStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  useChanged(() => setVersion((v) => v + 1));
  const startIso = isoDate(addDays(startOfWeek(today()), -(WEEKS - 1) * 7));
  const start = parseIsoDate(startIso);

  useEffect(() => {
    let live = true;
    api.projectStats(project.id, startIso, DAYS).then(
      (s) => {
        if (!live) return;
        setStats(s);
        setError(null);
      },
      (e) => live && setError(friendlyError(e)),
    );
    return () => {
      live = false;
    };
  }, [project.id, startIso, version]);

  const cur = data.cur.get(project.id) ?? 0;
  const prev = data.prev.get(project.id) ?? 0;
  const weekly = data.weekly.get(project.id) ?? new Array<number>(WEEKS).fill(0);
  const budget = data.budgets?.projects.find((b) => b.id === project.id);
  const dayHours = data.budgets?.dayHours ?? 8;
  const billed = data.billed?.get(project.id);
  const { labels, tipLabels } = weekLabels(data.periods);
  const left = budget ? weeksLeft(budget.budgetSeconds - budget.usedSeconds, weekly) : null;
  const tagName = (id: string | null) => data.tags.find((t) => t.id === id);

  return (
    <div className="space-y-3">
      <ErrorText>{error}</ErrorText>
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat
          label={capitalize(PERIOD_WORD[period])}
          value={formatDuration(cur)}
          hint={<Versus cur={cur} prev={prev} text={data.versus} />}
        />
        <Stat
          label="Bu ay çizelgede"
          value={billed !== undefined ? formatHours(billed) : "—"}
          hint={data.billed ? "Zaman çizelgesine yazılan" : "Bu ay çizelge kaydı yok"}
        />
        <Stat
          label="Ortalama odak bloğu"
          value={stats ? formatDuration(stats.focusSeconds) : "…"}
          hint={
            stats && stats.blocks > 0
              ? `${stats.blocks} blok · saatte ~${Math.round(stats.switchesPerHourX10 / 10)} uygulama geçişi`
              : "Son 26 hafta"
          }
        />
        <Stat
          label="Sözleşme bütçesi"
          value={budget ? `%${Math.round(budgetRatio(budget) * 100)}` : "—"}
          hint={
            budget
              ? `${formatDays(budget.usedSeconds, dayHours)} / ${formatDays(budget.budgetSeconds, dayHours)} adam-gün`
              : "Tanımlı değil"
          }
        />
      </div>

      <ChartCard title="Gün gün" note="Son 26 hafta · Pazartesi–Pazar">
        {stats ? (
          <div className="flex flex-wrap items-start gap-x-10 gap-y-4">
            <Heatmap start={start} days={stats.days} color={color} />
            <DaySummary start={start} days={stats.days} />
          </div>
        ) : (
          <div className="skeleton h-32 rounded-lg" />
        )}
      </ChartCard>

      <div className="grid gap-3 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <ChartCard title="Haftalık süre" note="Saat · soluk sütun süren hafta">
          <StackedBars
            labels={labels}
            tipLabels={tipLabels}
            series={[{ key: project.id, name: project.name, color, values: weekly }]}
            height={180}
            partial
          />
        </ChartCard>
        <ChartCard title="Günün hangi saatinde" note="Son 26 hafta">
          {stats ? (
            stats.totalSeconds > 0 ? (
              <>
                <HourBars hours={stats.hours} color={color} height={150} />
                <p className="mt-1.5 text-[11px] text-muted-foreground">{peakText(stats.hours)}</p>
              </>
            ) : (
              <p className="text-xs text-muted-foreground">Kayıt yok</p>
            )
          ) : (
            <div className="skeleton h-36 rounded-lg" />
          )}
        </ChartCard>
      </div>

      <div className="grid gap-3 md:grid-cols-2 lg:grid-cols-3">
        <ChartCard title="Uygulamalar" note="Son 26 hafta">
          {stats ? (
            <RankList items={stats.apps.map((a) => ({ key: a.appId, name: a.appName, color, value: a.seconds }))} />
          ) : (
            <div className="skeleton h-32 rounded-lg" />
          )}
        </ChartCard>
        <ChartCard title="Kategoriler" note="Son 26 hafta">
          {stats ? (
            <RankList
              items={stats.categories.map((c) => {
                const tag = tagName(c.id);
                return {
                  key: c.id ?? "none",
                  name: tag?.name ?? UNCATEGORIZED,
                  color: tagColor(tag),
                  value: c.seconds,
                };
              })}
            />
          ) : (
            <div className="skeleton h-32 rounded-lg" />
          )}
        </ChartCard>
        <ChartCard title="En çok açılan pencereler" note="Başlık">
          {stats ? (
            <RankList
              items={stats.titles.map((t, i) => ({
                key: `${i}`,
                name: t.title,
                sub: t.appName,
                title: `${t.appName} — ${t.title}`,
                color,
                value: t.seconds,
              }))}
            />
          ) : (
            <div className="skeleton h-32 rounded-lg" />
          )}
        </ChartCard>
      </div>

      {budget && (
        <ChartCard title="Bütçe tahmini" note="Adam-gün · kesikli çizgi son 4 haftanın hızı">
          <BurnChart
            periods={data.periods}
            weekly={weekly}
            usedSeconds={budget.usedSeconds}
            budgetSeconds={budget.budgetSeconds}
            dayHours={dayHours}
            color={color}
          />
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            {budget.usedSeconds >= budget.budgetSeconds
              ? `Bütçe ${formatDays(budget.usedSeconds - budget.budgetSeconds, dayHours)} adam-gün aşıldı.`
              : left === null
                ? "Son 4 haftada süre yok; bitiş tahmin edilemiyor."
                : `Bu hızla bütçe ~${Math.max(1, Math.round(left))} haftada, ${finishDate(addDays(today(), Math.round(left * 7)))} civarında doluyor.`}
          </p>
        </ChartCard>
      )}
    </div>
  );
}

const WEEKDAY_NAMES = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];
const dayFmt = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "long" });

/** Isı haritasının yanındaki özet: çalışılan gün, günlük ortalama, en uzun gün, en yoğun hafta günü. */
function DaySummary({ start, days }: { start: Date; days: number[] }) {
  const worked = days.filter((v) => v > 0);
  if (worked.length === 0) return <p className="text-xs text-muted-foreground">Son 26 haftada kayıt yok.</p>;
  const elapsed = Math.min(days.length, Math.floor((today().getTime() - start.getTime()) / 86_400_000) + 1);
  const best = days.indexOf(Math.max(...days));
  const byWeekday = [0, 1, 2, 3, 4, 5, 6].map((w) => days.filter((_, i) => i % 7 === w).reduce((a, b) => a + b, 0));
  const busiest = byWeekday.indexOf(Math.max(...byWeekday));
  const rows: [string, string][] = [
    ["Çalışılan gün", `${worked.length} / ${elapsed}`],
    ["Çalışılan günde ortalama", formatDuration(Math.round(worked.reduce((a, b) => a + b, 0) / worked.length))],
    ["En uzun gün", `${dayFmt.format(addDays(start, best))} · ${formatDuration(days[best])}`],
    ["En yoğun gün", WEEKDAY_NAMES[busiest]],
  ];
  return (
    <dl className="grid grid-cols-[auto_auto] gap-x-6 gap-y-2 text-xs">
      {rows.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-muted-foreground">{k}</dt>
          <dd className="tabular">{v}</dd>
        </div>
      ))}
    </dl>
  );
}

/** En yoğun iki saatlik dilim: "En çok 10:00–12:00 arası (%28)". */
function peakText(hours: number[]): string {
  const total = hours.reduce((a, b) => a + b, 0);
  let best = 0;
  for (let h = 1; h < 23; h++) if (hours[h] + hours[h + 1] > hours[best] + hours[best + 1]) best = h;
  const pct = Math.round(((hours[best] + hours[best + 1]) / (total || 1)) * 100);
  const hh = (h: number) => `${String(h).padStart(2, "0")}:00`;
  return `En yoğun dilim ${hh(best)}–${hh(best + 2)} (%${pct}).`;
}

const yearFmt = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "short", year: "numeric" });
/** Tahmini bitiş: bu yıl içindeyse gün ve ay, değilse yılıyla. */
function finishDate(d: Date): string {
  return d.getFullYear() === today().getFullYear() ? formatDate(d) : yearFmt.format(d);
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
