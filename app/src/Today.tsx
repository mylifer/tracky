import { useEffect, useState } from "react";
import { api, formatDuration, type AppStatus, type TrackingStatus, type UsageTotal } from "./api";

type Props = { initial: AppStatus; onChange: () => void };

/** Bir uygulamada en çok zaman geçen bu kadar başlık gösterilir. */
const MAX_TITLES = 15;

export default function Today({ initial, onChange }: Props) {
  const [tracking, setTracking] = useState<TrackingStatus>(initial.tracking);
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [titles, setTitles] = useState<UsageTotal[]>([]);
  const [diag, setDiag] = useState<string[] | null>(null);

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

  // Açık uygulamanın başlıkları da canlı güncellensin.
  useEffect(() => {
    if (!open) return;
    api.appTitles(open).then(setTitles);
  }, [open, apps]);

  async function togglePause() {
    await api.setPaused(!tracking.paused);
    setTracking({ ...tracking, paused: !tracking.paused });
    onChange();
  }

  async function runDiagnostics() {
    const lines: string[] = [];
    setDiag(lines);
    for (let i = 0; i < 5; i++) {
      lines.push(await api.diagnose());
      setDiag([...lines]);
      await new Promise((r) => setTimeout(r, 1000));
    }
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
          <span className="now-text">
            <strong>{tracking.current.appName}</strong>
            <span className="muted">
              {" — "}
              {tracking.current.title || "(pencere başlığı okunamadı)"}
            </span>
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
              <li key={a.key} className={open === a.key ? "open" : ""}>
                <button
                  className="row"
                  onClick={() => setOpen(open === a.key ? null : a.key)}
                  aria-expanded={open === a.key}
                >
                  <span className="name">
                    <span className="chevron">›</span>
                    {a.label}
                  </span>
                  <span className="bar">
                    <span style={{ width: `${Math.max(2, (a.seconds / max) * 100)}%` }} />
                  </span>
                  <span className="time">{formatDuration(a.seconds)}</span>
                </button>
                {open === a.key && (
                  <ul className="titles">
                    {titles.slice(0, MAX_TITLES).map((t) => (
                      <li key={t.key}>
                        <span className="title" title={t.label}>
                          {t.label || <em className="muted">(başlıksız)</em>}
                        </span>
                        <span className="time">{formatDuration(t.seconds)}</span>
                      </li>
                    ))}
                    {titles.length > MAX_TITLES && (
                      <li className="muted">+{titles.length - MAX_TITLES} başlık daha</li>
                    )}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="diag">
        <button className="ghost small" onClick={runDiagnostics}>
          Tanılama
        </button>
        {diag && (
          <>
            <p className="muted">5 saniye boyunca farklı pencerelere geç; sonra bu metni kopyala.</p>
            <pre>{diag.join("\n")}</pre>
          </>
        )}
      </section>
    </main>
  );
}
