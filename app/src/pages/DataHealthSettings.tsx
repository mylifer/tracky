import { useState } from "react";
import { api, type DataHealth } from "../api";
import { ErrorText, SettingRow } from "../components/settings";
import { Button } from "../components/ui/button";
import { friendlyError, notifyChanged, toast } from "../lib/feedback";

const when = new Intl.DateTimeFormat("tr-TR", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

/** Uygulama kimliğinden okunur ad: paket kimliği ya da exe yolunun son parçası. */
function appName(id: string) {
  if (id === "kum.idle") return "Boşta";
  const exe = id.split(/[\\/]/).pop() ?? id;
  return (
    exe
      .replace(/\.exe$/i, "")
      .split(".")
      .pop() ?? exe
  );
}

/** Bulguların okunur satırları (en yeniler önce, en çok `max`). */
function lines(h: DataHealth, max = 6): string[] {
  const out: { at: string; text: string }[] = [
    ...h.longCalls.map((c) => ({
      at: c.startedAt,
      text: `${when.format(new Date(c.startedAt))}: ${appName(c.appId)} görüşmesi ${when.format(new Date(c.endedAt))}'e kadar sürmüş; bilgisayar başında olunan son an ${when.format(new Date(c.until))}`,
    })),
    ...h.overlaps.map((o) => ({
      at: o.start,
      text: `${when.format(new Date(o.start))}: ${appName(o.firstApp)} ile ${appName(o.secondApp)} ${o.seconds} sn üst üste${o.repairable ? "" : " (aktarılmış satırda ya da sürüyor; onarılmaz)"}`,
    })),
    ...h.doubleRows.map((r) => ({
      at: r.date,
      text: `${r.date}: iki çizelge satırı aynı ${r.minutes} dk işi kapsıyor (çizelgede birini düzelt)`,
    })),
  ];
  return out
    .sort((a, b) => b.at.localeCompare(a.at))
    .slice(0, max)
    .map((l) => l.text);
}

/**
 * Ayarlar → Hakkında: veri denetimi. Üst üste binen oturumları, bilgisayar başında değilken de
 * süren görüşmeleri ve aynı işi kapsayan çizelge satırlarını bulur; onarılabilenleri (önce yedek
 * alınarak) onarır.
 */
export default function DataHealthSettings() {
  const [health, setHealth] = useState<DataHealth | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function check() {
    setBusy(true);
    setError(null);
    try {
      setHealth(await api.dataHealth());
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  async function repair() {
    setBusy(true);
    setError(null);
    try {
      const done = await api.repairData();
      notifyChanged();
      toast(`Onarıldı: ${done.sessions} oturum, ${done.calls} görüşme. Önce yedek alındı.`, { tone: "success" });
      setHealth(await api.dataHealth());
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  const repairable = health ? health.overlaps.filter((o) => o.repairable).length + health.longCalls.length : 0;
  const total = health ? health.overlaps.length + health.longCalls.length + health.doubleRows.length : 0;
  const hint = !health
    ? "Kayıtlarda olmaması gerekenleri arar: aynı bilgisayarda üst üste binen oturumlar, bilgisayar başında değilken de süren görüşmeler, aynı işi kapsayan çizelge satırları."
    : total === 0
      ? "Sorun bulunmadı."
      : `${total} bulgu${repairable ? `, ${repairable} tanesi onarılabilir` : ""}.`;
  return (
    <>
      <SettingRow label="Veri denetimi" hint={hint}>
        <div className="flex gap-2">
          {repairable > 0 && (
            <Button size="sm" disabled={busy} onClick={repair}>
              Onar ({repairable})
            </Button>
          )}
          <Button variant="outline" size="sm" disabled={busy} onClick={check}>
            {busy ? "Denetleniyor…" : "Denetle"}
          </Button>
        </div>
      </SettingRow>
      {(error || (health && total > 0)) && (
        <div className="space-y-1 px-4 py-2.5">
          <ErrorText>{error}</ErrorText>
          {health && (
            <ul className="list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
              {lines(health).map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </>
  );
}
