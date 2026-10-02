import { useCallback, useEffect, useMemo, useState } from "react";
import { api, formatDuration, type Report, type TrackingStatus } from "../api";
import { addDays, parseIsoDate } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { AppList, BucketList, Legend } from "./Breakdown";
import Timeline from "./Timeline";
import WeekChart from "./WeekChart";

type Props = {
  mode: "day" | "week";
  start: string;
  title: string;
  onPrev: () => void;
  onNext: () => void;
  onToday: (() => void) | null;
  onSelectDay: (iso: string) => void;
  tracking: TrackingStatus;
  onTogglePause: () => void;
};

export default function ReportView(p: Props) {
  const days = p.mode === "day" ? 1 : 7;
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api.report(p.start, days, p.mode === "day").then(
      (r) => {
        setReport(r);
        setError(null);
      },
      (e) => setError(String(e)),
    );
  }, [p.start, days, p.mode]);

  useEffect(load, [load]);

  // Aralık bugünü içeriyorsa takip güncellendikçe yenile.
  const end = addDays(parseIsoDate(p.start), days);
  const isLive = new Date() < end && new Date() >= parseIsoDate(p.start);
  useEffect(() => {
    if (!isLive) return;
    const unlisten = api.onStatus(load);
    return () => {
      unlisten.then((f) => f());
    };
  }, [isLive, load]);

  const tags = useMemo(() => tagMap(report?.tags ?? []), [report]);
  const categories = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "category"), [report]);
  // Efsane ve yığın sırası: etiketlerin kendi sırası, kategorisiz en sonda.
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);
  const hasProjects = (report?.tags ?? []).some((t) => t.kind === "project");

  return (
    <div className="page">
      <header className="report-head">
        <div>
          <div className="nav">
            <button className="ghost icon" onClick={p.onPrev} aria-label="Önceki">
              ‹
            </button>
            <h1>{p.title}</h1>
            <button className="ghost icon" onClick={p.onNext} disabled={isLive} aria-label="Sonraki">
              ›
            </button>
            {p.onToday && (
              <button className="ghost small" onClick={p.onToday}>
                {p.mode === "day" ? "Bugüne dön" : "Bu haftaya dön"}
              </button>
            )}
          </div>
          <p className="total">{formatDuration(report?.totalSeconds ?? 0)}</p>
        </div>
        {isLive && (
          <button className={p.tracking.paused ? "primary" : "ghost"} onClick={p.onTogglePause}>
            {p.tracking.paused ? "Devam Et" : "Duraklat"}
          </button>
        )}
      </header>

      {isLive && <NowCard tracking={p.tracking} />}
      {error && <p className="error">{error}</p>}

      {report && (
        <>
          <section className="card">
            <div className="card-head">
              <h2>{p.mode === "day" ? "Zaman çizelgesi" : "Günlere göre"}</h2>
              {order.length > 0 && <Legend order={order} tags={tags} />}
            </div>
            {p.mode === "day" ? (
              report.timeline.length ? (
                <Timeline from={parseIsoDate(p.start)} segments={report.timeline} tags={tags} />
              ) : (
                <p className="muted">Bu gün için kayıt yok.</p>
              )
            ) : (
              <WeekChart days={report.days} tags={tags} order={order} onSelectDay={p.onSelectDay} />
            )}
          </section>

          {report.totalSeconds > 0 && (
            <div className="grid-2">
              <section className="card">
                <h2>Kategoriler</h2>
                <BucketList buckets={report.categories} tags={tags} total={report.totalSeconds} kind="category" />
              </section>
              <section className="card">
                <h2>Projeler</h2>
                {hasProjects ? (
                  <BucketList buckets={report.projects} tags={tags} total={report.totalSeconds} kind="project" />
                ) : (
                  <p className="muted">
                    Henüz proje yok. Kategoriler sayfasından pencere başlığına göre proje ekleyebilirsin
                    (örn. başlıkta "fintrack" geçenler).
                  </p>
                )}
              </section>
            </div>
          )}

          {report.apps.length > 0 && (
            <section className="card">
              <h2>Uygulamalar</h2>
              <AppList
                apps={report.apps}
                tags={tags}
                categories={categories}
                start={p.start}
                days={days}
                onChanged={load}
              />
            </section>
          )}
        </>
      )}
    </div>
  );
}

function NowCard({ tracking }: { tracking: TrackingStatus }) {
  return (
    <section className="card now">
      <span className={`dot ${tracking.current && !tracking.paused ? "live" : ""}`} />
      {tracking.paused ? (
        <span className="muted">Takip duraklatıldı</span>
      ) : tracking.needsPermission ? (
        <span className="error">Erişilebilirlik izni gerekli — Ayarlar'dan kontrol et</span>
      ) : tracking.current ? (
        <span className="now-text">
          <strong>{tracking.current.appName}</strong>
          <span className="muted"> — {tracking.current.title || "(pencere başlığı okunamadı)"}</span>
        </span>
      ) : (
        <span className="muted">Boşta</span>
      )}
    </section>
  );
}

