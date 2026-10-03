import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type Report, type Tag } from "../api";
import { addDays, addMonths, daysInMonth, isoDate, parseIsoDate, today } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { AppList, Legend } from "./Breakdown";
import { DayCalendar, WeekCalendar } from "./Calendar";
import { IconLeft, IconRight } from "./Icons";
import MonthCalendar from "./MonthCalendar";
import Summary from "./Summary";

export type Mode = "day" | "week" | "month";

const MODES: { id: Mode; label: string; current: string; summary: string }[] = [
  { id: "day", label: "Gün", current: "Bugün", summary: "Gün" },
  { id: "week", label: "Hafta", current: "Bu hafta", summary: "Hafta" },
  { id: "month", label: "Ay", current: "Bu ay", summary: "Ay" },
];

type Props = {
  mode: Mode;
  start: string;
  title: string;
  onMode: (mode: Mode) => void;
  onPrev: () => void;
  onNext: () => void;
  onToday: (() => void) | null;
  onSelectDay: (iso: string) => void;
};

export default function ReportView(p: Props) {
  const days = p.mode === "day" ? 1 : p.mode === "week" ? 7 : daysInMonth(parseIsoDate(p.start));
  // Ay görünümünde zaman çizelgesi gerekmez (yalnızca gün toplamları).
  const timeline = p.mode !== "month";
  const [report, setReport] = useState<Report | null>(null);
  const [previous, setPrevious] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dailyHours, setDailyHours] = useState(8);
  useEffect(() => {
    api.goals().then((g) => setDailyHours(g.dailyHours), () => {});
  }, []);

  const load = useCallback(() => {
    api.report(p.start, days, timeline).then(
      (r) => {
        setReport(r);
        setError(null);
      },
      (e) => setError(String(e)),
    );
    const start = parseIsoDate(p.start);
    const prevStart = p.mode === "month" ? addMonths(start, -1) : addDays(start, -days);
    // Süren dönem önceki dönemin aynı uzunluktaki başıyla kıyaslanır
    // (ayın 3'ünde geçen ayın tamamıyla değil, ilk 3 günüyle).
    const elapsed = Math.round((+today() - +start) / 86_400_000) + 1;
    const fullPrev = p.mode === "month" ? daysInMonth(prevStart) : days;
    const prevDays = elapsed > 0 && elapsed < days ? Math.min(elapsed, fullPrev) : fullPrev;
    api.report(isoDate(prevStart), prevDays, false).then(setPrevious, () => setPrevious(null));
  }, [p.start, p.mode, days, timeline]);

  useEffect(load, [load]);
  useEffect(() => {
    const unlisten = api.onSync(load);
    return () => {
      unlisten.then((f) => f());
    };
  }, [load]);

  const from = parseIsoDate(p.start);
  const end = addDays(from, days);
  const isLive = new Date() < end && new Date() >= from;
  useEffect(() => {
    if (!isLive) return;
    const unlisten = api.onStatus(() => api.report(p.start, days, timeline).then(setReport));
    return () => {
      unlisten.then((f) => f());
    };
  }, [isLive, p.start, days, timeline]);

  const tags = useMemo(() => tagMap(report?.tags ?? []), [report]);
  const categories = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "category"), [report]);
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);

  const mode = MODES.find((m) => m.id === p.mode) ?? MODES[0];

  return (
    <div className="report">
      <header className="topbar" data-tauri-drag-region>
        <div data-tauri-drag-region className="topbar-spacer" />
        <h1 data-tauri-drag-region>{p.title}</h1>
        <div className="seg-tabs big" role="tablist" aria-label="Görünüm">
          {MODES.map((m) => (
            <button key={m.id} role="tab" aria-selected={p.mode === m.id} className={p.mode === m.id ? "on" : ""} onClick={() => p.onMode(m.id)}>
              {m.label}
            </button>
          ))}
        </div>
        <div className="nav">
          <button className="icon-btn" onClick={p.onPrev} aria-label="Önceki">
            <IconLeft />
          </button>
          <button className="pill" onClick={p.onToday ?? undefined} disabled={!p.onToday}>
            {mode.current}
          </button>
          <button className="icon-btn" onClick={p.onNext} disabled={isLive} aria-label="Sonraki">
            <IconRight />
          </button>
        </div>
      </header>

      {error && <p className="error pad">{error}</p>}
      {report && (
        <div className="report-grid">
          <div className="report-main">
            <section className="panel cal-panel">
              {order.length > 0 && <Legend order={order} tags={tags} />}
              {p.mode === "month" ? (
                <MonthCalendar from={from} days={report.days} tags={tags} dailyHours={dailyHours} onSelectDay={p.onSelectDay} />
              ) : report.totalSeconds === 0 ? (
                <Empty />
              ) : p.mode === "day" ? (
                <DayCalendar from={from} blocks={report.focus.blocks} segments={report.timeline} tags={tags} />
              ) : (
                <WeekCalendar
                  from={from}
                  blocks={report.focus.blocks}
                  dayTotals={report.days.map((d) => d.seconds)}
                  tags={tags}
                  onSelectDay={p.onSelectDay}
                />
              )}
            </section>
            {report.apps.length > 0 && (
              <section className="panel">
                <h2>Uygulamalar ve pencereler</h2>
                <AppList apps={report.apps} tags={tags} categories={categories as Tag[]} start={p.start} days={days} onChanged={load} />
              </section>
            )}
          </div>
          <Summary
            report={report}
            previous={previous}
            tags={tags}
            days={days}
            dailyHours={dailyHours}
            mode={p.mode}
            title={`Özet · ${isLive ? mode.current : mode.summary}`}
          />
        </div>
      )}
    </div>
  );
}

function Empty() {
  return (
    <div className="empty">
      <div className="empty-glyph">⌛</div>
      <p>Bu aralık için kayıt yok.</p>
      <p className="muted">Kum arka planda çalışırken takvim kendiliğinden dolacak.</p>
    </div>
  );
}
