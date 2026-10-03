import { useState } from "react";
import { TrendingDown, TrendingUp } from "lucide-react";
import type { Report, Tag } from "../api";
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
};

/** Sağ panel: süre, hedef, kırılım ve odak metrikleri. */
export default function Summary({ report, previous, tags, days, mode, title, dailyHours }: Props) {
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
  if (before === undefined || before === 0) return <div className="mt-1.5 text-[11px] text-muted-foreground">—</div>;
  const pct = Math.round(((now - before) / before) * 100);
  const up = pct >= 0;
  const Icon = up ? TrendingUp : TrendingDown;
  return (
    <div className={cn("mt-1.5 flex items-center gap-1 text-[11px] font-medium tabular", up ? "text-success" : "text-destructive")}>
      <Icon className="size-3.5" />
      {up ? "+" : ""}
      {pct}% <span className="font-normal text-muted-foreground">{unit}</span>
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
        {items.length === 0 ? (
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
            <span key={p.label} className="h-full first:rounded-l-full last:rounded-r-full" style={{ flexGrow: p.secs, background: p.color }} />
          ))}
      </div>
      <ul className="grid grid-cols-3 gap-2">
        {parts.map((p) => (
          <li key={p.label} className="min-w-0">
            <div className="flex items-center gap-1.5 text-[11px]">
              <i className="size-2 shrink-0 rounded-full" style={{ background: p.color }} />
              <span className="truncate">{p.label}</span>
            </div>
            <div className="mt-0.5 pl-3.5 text-[11px] text-muted-foreground tabular">{formatDuration(p.secs)}</div>
          </li>
        ))}
      </ul>
    </>
  );
}
