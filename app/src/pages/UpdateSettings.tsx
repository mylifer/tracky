import { useState } from "react";
import { api } from "../api";
import { useUpdate } from "../lib/useUpdate";

const time = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

export default function UpdateSettings() {
  const [status, setStatus] = useUpdate();
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!status) return null;

  const describe = () => {
    if (status.checking) return "Denetleniyor…";
    if (status.ready) return `Kum ${status.available} indirildi, kuruluma hazır.`;
    if (status.error) return `Denetlenemedi: ${status.error}`;
    if (status.lastChecked) return `Güncel · son denetim ${time.format(new Date(status.lastChecked))}`;
    return "Güncellemeler arka planda otomatik denetlenir.";
  };

  async function install() {
    setInstalling(true);
    setError(null);
    try {
      await api.installUpdate();
    } catch (e) {
      setError(String(e));
      setInstalling(false);
    }
  }

  return (
    <section className="card settings">
      <h2>Güncellemeler</h2>
      <div className="setting">
        <div>
          <strong>Kum {status.current}</strong>
          <p className="muted">{error ?? describe()}</p>
          {status.ready && status.notes && <p className="hint">{status.notes}</p>}
        </div>
        {status.ready ? (
          <button className="primary" disabled={installing} onClick={install}>
            {installing ? "Kuruluyor…" : "Yükle ve yeniden başlat"}
          </button>
        ) : (
          <button disabled={status.checking} onClick={() => api.checkUpdate().then(setStatus, (e) => setError(String(e)))}>
            Şimdi denetle
          </button>
        )}
      </div>
    </section>
  );
}
