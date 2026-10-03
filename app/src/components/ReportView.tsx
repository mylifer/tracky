import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type Report, type Tag } from "../api";
import { addDays, isoDate, parseIsoDate } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { AppList, Legend } from "./Breakdown";
import { DayCalendar, WeekCalendar } from "./Calendar";
import { IconLeft, IconRight } from "./Icons";
import Summary from "./Summary";

type Props = {
  mode: "day" | "week";
  start: string;
  title: string;
  onMode: (mode: "day" | "week") => void;
  onPrev: () => void;
  onNext: () => void;
  onToday: (() => void) | null;
  onSelectDay: (iso: string) => void;
};

export default function ReportView(p: Props) {
  const days = p.mode === "day" ? 1 : 7;
  const [report, setReport] = useState<Report | null>(null);
  const [previous, setPrevious] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api.report(p.start, days, true).then(
      (r) => {
        setReport(r);
        setError(null);
      },
      (e) => setError(String(e)),
    );
    const prev = isoDate(addDays(parseIsoDate(p.start), -days));
    api.report(prev, days, false).then(setPrevious, () => setPrevious(null));
  }, [p.start, days]);

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
    const unlisten = api.onStatus(() => api.report(p.start, days, true).then(setReport));
    return () => {
      unlisten.then((f) => f());
    };
  }, [isLive, p.start, days]);

  const tags = useMemo(() => tagMap(report?.tags ?? []), [report]);
  const categories = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "category"), [report]);
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);

  return (
    <div className="report">
      <header className="topbar" data-tauri-drag-region>
        <div data-tauri-drag-region className="topbar-spacer" />
        <h1 data-tauri-drag-region>{p.title}</h1>
        <div className="seg-tabs big" role="tablist" aria-label="Görünüm">
          <button role="tab" aria-selected={p.mode === "day"} className={p.mode === "day" ? "on" : ""} onClick={() => p.onMode("day")}>
            Gün
          </button>
          <button role="tab" aria-selected={p.mode === "week"} className={p.mode === "week" ? "on" : ""} onClick={() => p.onMode("week")}>
            Hafta
          </button>
        </div>
        <div className="nav">
          <button className="icon-btn" onClick={p.onPrev} aria-label="Önceki">
            <IconLeft />
          </button>
          <button className="pill" onClick={p.onToday ?? undefined} disabled={!p.onToday}>
            {p.mode === "day" ? "Bugün" : "Bu hafta"}
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
              {report.totalSeconds === 0 ? (
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
            title={p.mode === "day" ? (isLive ? "Özet · Bugün" : "Özet · Gün") : "Özet · Hafta"}
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
