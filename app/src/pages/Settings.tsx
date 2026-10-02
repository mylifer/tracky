import { useEffect, useMemo, useState } from "react";
import { api, type AppStatus, type PrivacySettings, type UsageTotal } from "../api";
import SyncSettings from "./SyncSettings";

export default function Settings({ status, onChange }: { status: AppStatus; onChange: () => void }) {
  const [privacy, setPrivacy] = useState<PrivacySettings | null>(null);
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [diag, setDiag] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.privacy().then(setPrivacy);
    api.knownApps().then(setApps);
  }, []);

  async function save(next: PrivacySettings) {
    try {
      setError(null);
      await api.savePrivacy(next);
      setPrivacy(next);
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleAutostart() {
    await api.setAutostart(!status.autostart);
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

  return (
    <div className="page">
      <header>
        <h1>Ayarlar</h1>
      </header>
      {error && <p className="error">{error}</p>}

      <section className="card settings">
        <h2>Genel</h2>
        <Toggle
          label="Bilgisayar açılınca başlat"
          hint="Kum menü çubuğunda sessizce başlar."
          checked={status.autostart}
          onChange={toggleAutostart}
        />
        {status.platform === "macos" && (
          <div className="setting">
            <div>
              <strong>Erişilebilirlik izni</strong>
              <p className="muted">Pencere başlıklarını okumak için gerekir.</p>
            </div>
            {status.accessibility ? (
              <span className="badge ok">Verildi</span>
            ) : (
              <button onClick={() => api.openAccessibilitySettings()}>Ayarları Aç</button>
            )}
          </div>
        )}
      </section>

      {privacy && (
        <section className="card settings">
          <h2>Gizlilik</h2>
          <Toggle
            label="Gizli pencerelerin başlığını kaydetme"
            hint="Tarayıcıların gizli/InPrivate pencerelerinde süre kaydedilir, başlık “Gizli” olarak saklanır."
            checked={privacy.hide_private_windows}
            onChange={() => save({ ...privacy, hide_private_windows: !privacy.hide_private_windows })}
          />
          <AppPicker
            title="Hiç kaydedilmeyen uygulamalar"
            hint="Bu uygulamalarda geçen süre hiç kaydedilmez (örn. şifre yöneticileri)."
            selected={privacy.excluded_apps}
            apps={apps}
            onChange={(excluded_apps) => save({ ...privacy, excluded_apps })}
          />
          <AppPicker
            title="Başlığı kaydedilmeyen uygulamalar"
            hint="Süre kaydedilir ama pencere başlığı “Gizli” olarak saklanır (örn. e-posta)."
            selected={privacy.hidden_title_apps}
            apps={apps}
            onChange={(hidden_title_apps) => save({ ...privacy, hidden_title_apps })}
          />
          <p className="muted hint">Değişiklikler yeni kayıtlara uygulanır; geçmiş kayıtlar değişmez.</p>
        </section>
      )}

      <SyncSettings />

      <section className="card settings">
        <h2>Sorun giderme</h2>
        <div className="setting">
          <div>
            <strong>Tanılama</strong>
            <p className="muted">
              Pencere başlıkları görünmüyorsa çalıştır, 5 saniye boyunca farklı pencerelere geç ve
              çıkan metni paylaş.
            </p>
          </div>
          <button onClick={runDiagnostics}>Çalıştır</button>
        </div>
        {diag && <pre className="diag-out">{diag.join("\n")}</pre>}
      </section>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: () => void;
}) {
  return (
    <label className="setting">
      <div>
        <strong>{label}</strong>
        <p className="muted">{hint}</p>
      </div>
      <span className="switch">
        <input type="checkbox" checked={checked} onChange={onChange} />
        <span />
      </span>
    </label>
  );
}

function AppPicker({
  title,
  hint,
  selected,
  apps,
  onChange,
}: {
  title: string;
  hint: string;
  selected: string[];
  apps: UsageTotal[];
  onChange: (ids: string[]) => void;
}) {
  const [pick, setPick] = useState("");
  const names = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);
  const available = apps.filter((a) => !selected.includes(a.key));
  return (
    <div className="setting column">
      <div>
        <strong>{title}</strong>
        <p className="muted">{hint}</p>
      </div>
      <div className="chips">
        {selected.map((id) => (
          <span key={id} className="chip" title={id}>
            {names.get(id) ?? id}
            <button
              className="ghost icon small"
              onClick={() => onChange(selected.filter((s) => s !== id))}
              aria-label="Kaldır"
            >
              ×
            </button>
          </span>
        ))}
        {selected.length === 0 && <span className="muted">Yok</span>}
      </div>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (pick) onChange([...selected, pick]);
          setPick("");
        }}
      >
        <select value={pick} onChange={(e) => setPick(e.target.value)} aria-label="Uygulama seç">
          <option value="">Uygulama seç…</option>
          {available.map((a) => (
            <option key={a.key} value={a.key}>
              {a.label}
            </option>
          ))}
        </select>
        <button type="submit" disabled={!pick}>
          Ekle
        </button>
      </form>
    </div>
  );
}
