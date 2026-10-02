import { useEffect, useState } from "react";
import { api, formatDuration, type AppStatus, type TrackingStatus, type UsageTotal } from "./api";

type Props = { initial: AppStatus; onChange: () => void };

export default function Today({ initial, onChange }: Props) {
  const [tracking, setTracking] = useState<TrackingStatus>(initial.tracking);
  const [apps, setApps] = useState<UsageTotal[]>([]);

  useEffect(() => {
    api.todayApps().then(setApps);
    const unlisten = api.onStatus((s) => {
      setTracking(s);
      api.todayApps().then(setApps);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function togglePause() {
    await api.setPaused(!tracking.paused);
    setTracking({ ...tracking, paused: !tracking.paused });
    onChange();
  }

  const max = apps[0]?.seconds ?? 1;
  return (
    <main className="page">
      <header>
        <div>
          <h1>Bugün</h1>
          <p className="total">{formatDuration(tracking.todaySeconds)}</p>
        </div>
        <button className={tracking.paused ? "primary" : "ghost"} onClick={togglePause}>
          {tracking.paused ? "Devam Et" : "Duraklat"}
        </button>
      </header>

      <section className="card now">
        <span className={`dot ${tracking.current ? "live" : ""}`} />
        {tracking.paused ? (
          <span className="muted">Takip duraklatıldı</span>
        ) : tracking.current ? (
          <span>
            <strong>{tracking.current.appName}</strong>
            {tracking.current.title && <span className="muted"> — {tracking.current.title}</span>}
          </span>
        ) : (
          <span className="muted">Boşta</span>
        )}
      </section>

      <section className="card">
        {apps.length === 0 ? (
          <p className="muted">Henüz kayıt yok. Çalışmaya başladığında burada görünecek.</p>
        ) : (
          <ul className="apps">
            {apps.map((a) => (
              <li key={a.key}>
                <span className="name">{a.label}</span>
                <span className="bar">
                  <span style={{ width: `${Math.max(2, (a.seconds / max) * 100)}%` }} />
                </span>
                <span className="time">{formatDuration(a.seconds)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  );
}
