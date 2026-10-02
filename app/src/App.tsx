import { useCallback, useEffect, useState } from "react";
import { api, type AppStatus, type TrackingStatus } from "./api";
import ReportView from "./components/ReportView";
import { addDays, formatDay, formatWeek, isoDate, parseIsoDate, startOfWeek, today } from "./lib/dates";
import Onboarding from "./Onboarding";
import Categories from "./pages/Categories";
import Settings from "./pages/Settings";

type View = "day" | "week" | "categories" | "settings";

const NAV: { id: View; label: string }[] = [
  { id: "day", label: "Gün" },
  { id: "week", label: "Hafta" },
  { id: "categories", label: "Kategoriler" },
  { id: "settings", label: "Ayarlar" },
];

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

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand">
          <img src="/icon.png" alt="" />
          Kum
        </div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={`nav-item ${view === n.id ? "active" : ""}`}
            onClick={() => setView(n.id)}
            aria-current={view === n.id ? "page" : undefined}
          >
            {n.label}
          </button>
        ))}
      </nav>
      <main className="content">
        {view === "day" && (
          <ReportView
            mode="day"
            start={day}
            title={formatDay(parseIsoDate(day))}
            onPrev={() => setDay(shift(day, -1))}
            onNext={() => setDay(shift(day, 1))}
            onToday={day === todayIso ? null : () => setDay(todayIso)}
            onSelectDay={setDay}
            tracking={tracking}
            onTogglePause={togglePause}
          />
        )}
        {view === "week" && (
          <ReportView
            mode="week"
            start={week}
            title={formatWeek(parseIsoDate(week))}
            onPrev={() => setWeek(shift(week, -7))}
            onNext={() => setWeek(shift(week, 7))}
            onToday={week === thisWeek ? null : () => setWeek(thisWeek)}
            onSelectDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
            tracking={tracking}
            onTogglePause={togglePause}
          />
        )}
        {view === "categories" && <Categories />}
        {view === "settings" && <Settings status={status} onChange={refresh} />}
      </main>
    </div>
  );
}
