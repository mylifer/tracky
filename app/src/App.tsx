import { useCallback, useEffect, useState } from "react";
import { api, formatDuration, type AppStatus, type TrackingStatus } from "./api";
import { IconPause, IconPlay, IconSettings, IconTag, IconToday, IconWeek } from "./components/Icons";
import ReportView from "./components/ReportView";
import { addDays, formatWeek, isoDate, parseIsoDate, startOfWeek, today } from "./lib/dates";
import Onboarding from "./Onboarding";
import Categories from "./pages/Categories";
import Settings from "./pages/Settings";

type View = "day" | "week" | "categories" | "settings";

const NAV: { id: View; label: string; icon: React.ReactNode }[] = [
  { id: "day", label: "Takvim", icon: <IconToday /> },
  { id: "week", label: "Hafta", icon: <IconWeek /> },
  { id: "categories", label: "Kategoriler", icon: <IconTag /> },
];

const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.status().then(setStatus, (e) => setError(String(e)));
  }, []);

  useEffect(refresh, [refresh]);

  if (error)
    return (
      <main className="center">
        <p className="error">{error}</p>
      </main>
    );
  if (!status) return null;
  if (!status.onboarded || !status.accessibility) return <Onboarding status={status} onChange={refresh} />;
  return <Shell status={status} refresh={refresh} />;
}

function Shell({ status, refresh }: { status: AppStatus; refresh: () => void }) {
  const [view, setView] = useState<View>("day");
  const [day, setDay] = useState(isoDate(today()));
  const [week, setWeek] = useState(isoDate(startOfWeek(today())));
  const [tracking, setTracking] = useState<TrackingStatus>(status.tracking);

  useEffect(() => {
    const unlisten = api.onStatus(setTracking);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function togglePause() {
    await api.setPaused(!tracking.paused);
    setTracking({ ...tracking, paused: !tracking.paused });
  }

  const shift = (iso: string, n: number) => isoDate(addDays(parseIsoDate(iso), n));
  const todayIso = isoDate(today());
  const thisWeek = isoDate(startOfWeek(today()));
  const dayDate = parseIsoDate(day);

  return (
    <div className={`shell ${status.platform === "macos" ? "mac" : ""}`}>
      <nav className="sidebar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <img src="/icon.png" alt="" />
          <span>KUM</span>
        </div>
        <div className="nav-list">
          {NAV.map((n) => (
            <button
              key={n.id}
              className={`nav-item ${view === n.id ? "active" : ""}`}
              onClick={() => setView(n.id)}
              aria-current={view === n.id ? "page" : undefined}
            >
              {n.icon}
              {n.label}
            </button>
          ))}
        </div>
        <div className="nav-list bottom">
          <button
            className={`nav-item ${view === "settings" ? "active" : ""}`}
            onClick={() => setView("settings")}
            aria-current={view === "settings" ? "page" : undefined}
          >
            <IconSettings />
            Ayarlar
          </button>
        </div>
        <LiveCard tracking={tracking} onToggle={togglePause} />
      </nav>
      <main className="content">
        {(view === "day" || view === "week") && (
          <ReportView
            mode={view}
            start={view === "day" ? day : week}
            title={view === "day" ? capitalize(longDate.format(dayDate)) : formatWeek(parseIsoDate(week))}
            onMode={(m) => {
              if (m === "week") setWeek(isoDate(startOfWeek(dayDate)));
              setView(m);
            }}
            onPrev={() => (view === "day" ? setDay(shift(day, -1)) : setWeek(shift(week, -7)))}
            onNext={() => (view === "day" ? setDay(shift(day, 1)) : setWeek(shift(week, 7)))}
            onToday={
              view === "day"
                ? day === todayIso
                  ? null
                  : () => setDay(todayIso)
                : week === thisWeek
                  ? null
                  : () => setWeek(thisWeek)
            }
            onSelectDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
          />
        )}
        {view === "categories" && <Categories />}
        {view === "settings" && <Settings status={status} onChange={refresh} />}
      </main>
    </div>
  );
}

/** Kenar çubuğunun altında: şu an ne takip ediliyor, bugünkü toplam, duraklat. */
function LiveCard({ tracking, onToggle }: { tracking: TrackingStatus; onToggle: () => void }) {
  const state = tracking.paused
    ? "Duraklatıldı"
    : tracking.needsPermission
      ? "İzin gerekli"
      : tracking.current
        ? tracking.current.appName
        : "Boşta";
  const live = !tracking.paused && !tracking.needsPermission && !!tracking.current;
  return (
    <div className="live-card">
      <div className="live-head">
        <span className={`pulse ${live ? "on" : ""}`} />
        <span className="live-state">{state}</span>
      </div>
      {live && tracking.current?.title && <div className="live-title">{tracking.current.title}</div>}
      <div className="live-foot">
        <span>
          <small>Bugün</small>
          {formatDuration(tracking.todaySeconds)}
        </span>
        <button className="icon-btn round" onClick={onToggle} aria-label={tracking.paused ? "Devam et" : "Duraklat"}>
          {tracking.paused ? <IconPlay size={15} /> : <IconPause size={15} />}
        </button>
      </div>
    </div>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
