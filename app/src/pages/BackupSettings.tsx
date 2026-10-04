import { useEffect, useState } from "react";
import { api, type BackupStatus, type PickedBackup } from "../api";
import { ErrorText, SettingBlock, SettingRow } from "../components/settings";
import { Button } from "../components/ui/button";
import { formatDate, formatTime } from "../lib/dates";
import { friendlyError } from "../lib/feedback";

function when(iso: string) {
  const d = new Date(iso);
  return `${formatDate(d)} ${formatTime(d)}`;
}

/** Ayarlar → Veriler: haftalık otomatik yedek, elle yedek ve yedekten geri yükleme. */
export default function BackupSettings() {
  const [status, setStatus] = useState<BackupStatus | null>(null);
  const [picked, setPicked] = useState<PickedBackup | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => api.backupStatus().then(setStatus, (e) => setError(friendlyError(e)));
  useEffect(() => {
    refresh();
  }, []);

  async function run(f: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  const last = status?.last ? `Son yedek: ${when(status.last)}.` : "Henüz yedek yok.";
  return (
    <>
      <SettingRow label="Yedekler" hint={`Haftada bir kendiliğinden alınır, en yeni 8 yedek saklanır. ${last}`}>
        <Button variant="ghost" size="sm" onClick={() => run(() => api.openBackupFolder())}>
          Klasörü aç
        </Button>
        <Button variant="outline" size="sm" disabled={busy} onClick={() => run(() => api.backupNow().then(refresh))}>
          Şimdi yedekle
        </Button>
      </SettingRow>
      {picked ? (
        <SettingBlock
          label="Bu yedek geri yüklensin mi?"
          hint="Kum yeniden başlar. Şimdiki veriler silinmez, yedek klasörüne “geri-yukleme-oncesi” adıyla taşınır. Senkronizasyon açıksa diğer bilgisayarlardaki daha yeni değişiklikler yeniden gelir."
        >
          <p className="text-xs break-all text-muted-foreground selectable">{picked.path}</p>
          <p className="text-xs">
            {picked.sessions.toLocaleString("tr-TR")} kayıt
            {picked.lastActivity && ` · son kayıt ${when(picked.lastActivity)}`}
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setPicked(null)}>
              Vazgeç
            </Button>
            <Button
              variant="destructive"
              size="sm"
              disabled={busy}
              onClick={() => run(() => api.restoreBackup(picked.path))}
            >
              Geri yükle ve yeniden başlat
            </Button>
          </div>
        </SettingBlock>
      ) : (
        <SettingRow label="Yedekten geri yükle" hint="Seçilen yedekteki kayıtlar şimdikilerin yerine geçer.">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => run(() => api.pickBackup().then(setPicked))}
          >
            Yedek seç…
          </Button>
        </SettingRow>
      )}
      {error && (
        <div className="px-4 py-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
    </>
  );
}
