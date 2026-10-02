import { useEffect, useState } from "react";
import { api, type SyncStatus } from "../api";
import { formatTime } from "../lib/dates";

/** Supabase bağlantısı, giriş ve eşitleme durumu. */
export default function SyncSettings() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.syncStatus().then(setStatus);
    const unlisten = api.onSync(setStatus);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function run(f: () => Promise<SyncStatus | void>) {
    setBusy(true);
    setError(null);
    try {
      const s = await f();
      if (s) setStatus(s);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;

  return (
    <section className="card settings">
      <h2>Senkronizasyon</h2>
      <p className="muted hint">
        Verilerini kendi Supabase projende saklayarak Mac ve Windows'ta birleşik rapor görürsün.
        Kurulum adımları README'de (supabase/migrations klasöründeki SQL'i bir kez çalıştır).
      </p>
      {error && <p className="error">{error}</p>}

      {!status.configured ? (
        <form
          className="stack-form"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.syncConfigure(url, key));
          }}
        >
          <label>
            Proje adresi
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://abcd.supabase.co" />
          </label>
          <label>
            Anon (publishable) anahtar
            <input value={key} onChange={(e) => setKey(e.target.value)} placeholder="eyJhbGciOi…" />
          </label>
          <button type="submit" className="primary" disabled={busy || !url || !key}>
            Bağlantıyı kaydet
          </button>
        </form>
      ) : !status.email ? (
        <form
          className="stack-form"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.syncSignIn(email, password, false));
          }}
        >
          <p className="muted">Bağlı proje: {status.url}</p>
          <label>
            E-posta
            <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username" />
          </label>
          <label>
            Şifre
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          </label>
          <div className="row-actions">
            <button type="submit" className="primary" disabled={busy || !email || password.length < 6}>
              Giriş yap
            </button>
            <button
              type="button"
              disabled={busy || !email || password.length < 6}
              onClick={() => run(() => api.syncSignIn(email, password, true))}
            >
              Hesap oluştur
            </button>
            <button type="button" className="ghost small" onClick={() => run(api.syncDisconnect)}>
              Bağlantıyı kaldır
            </button>
          </div>
        </form>
      ) : (
        <>
          <div className="setting">
            <div>
              <strong>{status.email}</strong>
              <p className="muted">
                {status.last
                  ? `${status.last.ok ? "Son eşitleme" : "Son deneme"} ${formatTime(new Date(status.last.at))} · ${status.last.message}`
                  : "Henüz eşitlenmedi"}
              </p>
            </div>
            <span className={`badge ${status.last && !status.last.ok ? "warn" : "ok"}`}>
              {status.last && !status.last.ok ? "Hata" : "Bağlı"}
            </span>
          </div>
          {status.last && !status.last.ok && <p className="error">{status.last.message}</p>}
          <div className="row-actions">
            <button disabled={busy} onClick={() => run(api.syncNow)}>
              Şimdi eşitle
            </button>
            <button className="ghost" onClick={() => run(api.syncSignOut)}>
              Çıkış yap
            </button>
          </div>
          <p className="muted hint">Her 5 dakikada bir otomatik eşitlenir.</p>
        </>
      )}
    </section>
  );
}
