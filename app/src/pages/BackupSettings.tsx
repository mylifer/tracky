import { useEffect, useState } from "react";
import { api, type BackupStatus, type PickedBackup, type Retention } from "../api";
import { ErrorText, SettingBlock, SettingRow } from "../components/settings";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
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
      {status?.damage && (
        <ErrorText>
          Veritabanında bozulma bulundu; yeni yedek alınmıyor, eski sağlam yedekler korunuyor. Aşağıdan son yedeği geri
          yükleyebilirsin. ({status.damage})
        </ErrorText>
      )}
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
      <RetentionRow onChanged={refresh} />
      {error && (
        <div className="px-4 py-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
    </>
  );
}

const RETENTION_CHOICES: { months: number | null; label: string }[] = [
  { months: null, label: "Kapalı" },
  { months: 6, label: "6 aydan eski" },
  { months: 12, label: "1 yıldan eski" },
  { months: 24, label: "2 yıldan eski" },
];

/**
 * Eski kayıtların sadeleştirilmesi: seçilen süreden eski pencere başlıkları ve adres yolları
 * silinir; süreler, uygulama, alan adı, kategori ve proje kalır (rapor toplamları değişmez).
 * Geri alınamaz: açmadan önce onay sorulur, açılınca önce yedek alınır.
 */
function RetentionRow({ onChanged }: { onChanged: () => void }) {
  const [r, setR] = useState<Retention | null>(null);
  const [pending, setPending] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.retention().then(setR, (e) => setError(friendlyError(e)));
  }, []);
  if (!r) return error ? <ErrorText>{error}</ErrorText> : null;

  const apply = async (months: number | null) => {
    setBusy(true);
    setError(null);
    setPending(null);
    try {
      setR(await api.setRetention(months));
      onChanged();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  };
  const choose = (v: string) => {
    const months = v === "off" ? null : Number(v);
    // Kapatmak ya da süreyi uzatmak hiçbir şeyi silmez; onay gerekmez.
    if (months === null || (r.months !== null && months >= r.months)) apply(months);
    else setPending(months);
  };
  const label = (m: number | null) => RETENTION_CHOICES.find((c) => c.months === m)?.label ?? "";
  const last = r.lastRun
    ? ` Son çalışma ${when(r.lastRun)}: ${r.lastCount.toLocaleString("tr-TR")} kayıt sadeleştirildi.`
    : "";

  return (
    <>
      <SettingRow
        label="Eski kayıtları sadeleştir"
        hint={`Pencere başlıkları ve adres yolları silinir; süreler, uygulama, site, kategori ve proje kalır, rapor toplamları değişmez. Aramada eski başlıklar bulunmaz. Yalnızca bu bilgisayarın ayarı; sadeleştirilen kayıtlar eşitlenir.${last}`}
      >
        <Select value={r.months === null ? "off" : String(r.months)} onValueChange={choose} disabled={busy}>
          <SelectTrigger size="sm" className="w-36" aria-label="Eski kayıtları sadeleştir">
            <SelectValue />
          </SelectTrigger>
          <SelectContent align="end">
            {RETENTION_CHOICES.map((c) => (
              <SelectItem key={c.label} value={c.months === null ? "off" : String(c.months)}>
                {c.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingRow>
      {pending !== null && (
        <SettingBlock
          label={`${label(pending)} kayıtlar sadeleştirilsin mi?`}
          hint="Geri alınamaz. Önce yedek alınır; başlıklara yeniden ihtiyaç olursa o yedekten geri yüklenebilir. Sonra günde bir kez kendiliğinden çalışır."
        >
          <div className="flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setPending(null)}>
              Vazgeç
            </Button>
            <Button variant="destructive" size="sm" disabled={busy} onClick={() => apply(pending)}>
              Yedekle ve sadeleştir
            </Button>
          </div>
        </SettingBlock>
      )}
      {error && (
        <div className="px-4 py-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
    </>
  );
}
