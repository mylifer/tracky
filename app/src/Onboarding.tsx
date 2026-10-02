import { useEffect, useState } from "react";
import { api, type AppStatus } from "./api";

type Props = { status: AppStatus; onChange: () => void };

export default function Onboarding({ status, onChange }: Props) {
  const [accessibility, setAccessibility] = useState(status.accessibility);
  const [autostart, setAutostart] = useState(status.autostart);
  const isMac = status.platform === "macos";

  // İzin Sistem Ayarları'nda verilir; geri dönüldüğünde fark etmek için yokla.
  useEffect(() => {
    if (accessibility) return;
    const id = setInterval(async () => {
      const s = await api.status();
      if (s.accessibility) setAccessibility(true);
    }, 1500);
    return () => clearInterval(id);
  }, [accessibility]);

  async function toggleAutostart() {
    const next = !autostart;
    await api.setAutostart(next);
    setAutostart(next);
  }

  async function finish() {
    await api.completeOnboarding();
    onChange();
  }

  return (
    <main className="center">
      <div className="card onboarding">
        <img src="/icon.png" alt="" className="logo" />
        <h1>Kum'a hoş geldin</h1>
        <p className="muted">
          Kum, hangi uygulamada ve pencerede ne kadar zaman geçirdiğini sessizce kaydeder.
          Veriler yalnızca bu bilgisayarda tutulur.
        </p>

        <ol className="steps">
          <li className={accessibility ? "done" : ""}>
            <div>
              <strong>Pencere başlıklarını okuma izni</strong>
              <p className="muted">
                {isMac
                  ? "Hangi pencerede olduğunu görmek için Erişilebilirlik izni gerekir. Sistem Ayarları'nda Kum'u etkinleştir."
                  : "Bu sistemde ek izin gerekmez."}
              </p>
            </div>
            {accessibility ? (
              <span className="badge ok">Verildi</span>
            ) : (
              <div className="actions">
                <button onClick={() => api.requestAccessibility()}>İzin İste</button>
                <button className="ghost" onClick={() => api.openAccessibilitySettings()}>
                  Ayarları Aç
                </button>
              </div>
            )}
          </li>
          <li className="done">
            <div>
              <strong>Bilgisayar açılınca başlat</strong>
              <p className="muted">Takip arka planda, menü çubuğunda devam eder.</p>
            </div>
            <label className="switch">
              <input type="checkbox" checked={autostart} onChange={toggleAutostart} />
              <span />
            </label>
          </li>
        </ol>

        <button className="primary" disabled={!accessibility} onClick={finish}>
          {accessibility ? "Başla" : "İzin bekleniyor…"}
        </button>
      </div>
    </main>
  );
}
