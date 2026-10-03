import { useState } from "react";
import { api } from "../api";
import { ErrorText, SettingRow, SettingsGroup } from "../components/settings";
import { Button } from "../components/ui/button";
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
    <SettingsGroup title="Güncellemeler">
      <SettingRow label={`Kum ${status.current}`} hint={describe()}>
        {status.ready ? (
          <Button size="sm" disabled={installing} onClick={install}>
            {installing ? "Kuruluyor…" : "Yükle ve yeniden başlat"}
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            disabled={status.checking}
            onClick={() => api.checkUpdate().then(setStatus, (e) => setError(String(e)))}
          >
            Şimdi denetle
          </Button>
        )}
      </SettingRow>
      {(error || (status.ready && status.notes)) && (
        <div className="space-y-1 px-4 py-2.5">
          <ErrorText>{error}</ErrorText>
          {status.ready && status.notes && <p className="text-xs whitespace-pre-wrap text-muted-foreground">{status.notes}</p>}
        </div>
      )}
    </SettingsGroup>
  );
}
