import { useState } from "react";
import type { Report, Tag } from "../api";
import { formatDuration } from "../api";
import { NO_PROJECT, UNCATEGORIZED, tagColor } from "../lib/tags";
import { ScoreRing, scoreLabel } from "./Stats";

type Props = {
  report: Report;
  previous: Report | null;
  tags: Map<string, Tag>;
  days: number;
  title: string;
  /** Ayarlardaki günlük hedef (saat). */
  dailyHours: number;
};

/** Sağ panel: Rize'deki "Summary" düzeni. */
export default function Summary({ report, previous, tags, days, title, dailyHours }: Props) {
  const f = report.focus;
  const target = dailyHours * 3600 * activeDays(report, days);
  const pctTarget = target ? Math.round((report.totalSeconds / target) * 100) : 0;
  const breakSecs = f.breakSeconds;
  const otherWork = Math.max(0, report.totalSeconds - f.focusSeconds);

  return (
    <aside className="summary">
      <div className="summary-head">
        <span>{title}</span>
      </div>

      <section className="s-card">
        <div className="kv-row">
          <div>
            <div className="kv-label">Çalışma süresi</div>
            <div className="kv-big">{formatDuration(report.totalSeconds)}</div>
            <Delta now={report.totalSeconds} before={previous?.totalSeconds} unit={days === 1 ? "düne göre" : "geçen haftaya göre"} />
          </div>
          <div className="kv-right">
            <div className="kv-label">Hedefin yüzdesi</div>
            <div className="kv-mid">
              %{pctTarget} <small>/ {formatDuration(target)}</small>
            </div>
          </div>
        </div>
      </section>

      <BreakdownCard report={report} tags={tags} />

      <div className="s-pair">
        <section className="s-card">
          <div className="kv-label">Odak skoru</div>
          <div className="score-line">
            <ScoreRing score={f.score} size={54} />
            <span className="kv-word">{scoreLabel(f.score)}</span>
          </div>
        </section>
        <section className="s-card">
          <div className="kv-label">Odak süresi</div>
          <div className="kv-mid">{formatDuration(f.focusSeconds)}</div>
          <Delta now={f.focusSeconds} before={previous?.focus.focusSeconds} unit={days === 1 ? "düne göre" : "geçen haftaya göre"} />
        </section>
      </div>

      <section className="s-card">
        <div className="kv-row tight">
          <div className="kv-label">Verimlilik metrikleri</div>
          <div className="kv-label">Toplam {formatDuration(report.totalSeconds + breakSecs)}</div>
        </div>
        <Metrics
          parts={[
            { label: "Odak", secs: f.focusSeconds, cls: "m-focus" },
            { label: "Diğer çalışma", secs: otherWork, cls: "m-work" },
            { label: "Mola", secs: breakSecs, cls: "m-break" },
          ]}
        />
        <div className="kv-foot">
          {(f.switchesPerHourX10 / 10).toLocaleString("tr-TR")} geçiş/sa · en uzun odak{" "}
          {formatDuration(f.longestFocusSeconds)}
        </div>
      </section>
    </aside>
  );
}

function activeDays(report: Report, days: number): number {
  if (days === 1) return 1;
  return Math.max(1, report.days.filter((d) => d.seconds > 0).length);
}

function Delta({ now, before, unit }: { now: number; before?: number; unit: string }) {
  if (before === undefined || before === 0) return <div className="delta muted">—</div>;
  const pct = Math.round(((now - before) / before) * 100);
  return (
    <div className={`delta ${pct >= 0 ? "up" : "down"}`}>
      {pct >= 0 ? "+" : ""}
      {pct}% {unit}
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
    <section className="s-card">
      <div className="seg-tabs" role="tablist">
        {(
          [
            ["categories", "Kategoriler"],
            ["projects", "Projeler"],
            ["apps", "Uygulamalar"],
          ] as [Tab, string][]
        ).map(([id, label]) => (
          <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {items.length === 0 ? (
        <p className="muted">Kayıt yok.</p>
      ) : (
        <div className="donut-row">
          <Donut parts={donut.map((d) => ({ value: d.secs, color: d.color }))} />
          <ul className="donut-legend">
            {donut.map((d) => (
              <li key={d.key}>
                <i style={{ background: d.color }} />
                <span className="dl-name">{d.name}</span>
                <span className="dl-time">{formatDuration(d.secs)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

/** Halka grafik; dilimler arasında 2px boşluk. */
function Donut({ parts, size = 92 }: { parts: { value: number; color: string }[]; size?: number }) {
  const total = parts.reduce((s, p) => s + p.value, 0) || 1;
  const r = size / 2 - 9;
  const c = 2 * Math.PI * r;
  const gap = parts.length > 1 ? 3 : 0;
  let offset = 0;
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="donut" aria-hidden="true">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--line)" strokeWidth={11} />
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
            strokeWidth={11}
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

function Metrics({ parts }: { parts: { label: string; secs: number; cls: string }[] }) {
  const total = parts.reduce((s, p) => s + p.secs, 0);
  return (
    <>
      <div className="metric-bar">
        {parts
          .filter((p) => p.secs > 0)
          .map((p) => (
            <span key={p.label} className={p.cls} style={{ flexGrow: p.secs }} />
          ))}
        {total === 0 && <span className="m-empty" />}
      </div>
      <ul className="metric-legend">
        {parts.map((p) => (
          <li key={p.label}>
            <span>
              <i className={p.cls} />
              {p.label}
            </span>
            <small>{formatDuration(p.secs)}</small>
          </li>
        ))}
      </ul>
    </>
  );
}
