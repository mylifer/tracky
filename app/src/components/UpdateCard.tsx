import { useState } from "react";
import { Download, LoaderCircle, TriangleAlert } from "lucide-react";
import { api, type UpdateStatus } from "../api";
import { Button } from "./ui/button";
import { friendlyError } from "../lib/feedback";

/**
 * Kenar çubuğunun altında yeni sürüm kartı: bulunur bulunmaz "indiriliyor", inince
 * doğrudan "Yükle ve yeniden başlat", indirilemezse "Tekrar dene". Güncel değilken görünmez.
 */
export function UpdateCard({ status, onStatus }: { status: UpdateStatus | null; onStatus: (s: UpdateStatus) => void }) {
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!status?.available) return null;

  const failed = !status.ready && !status.checking && !!status.error;
  const downloading = !status.ready && !failed;

  async function install() {
    setInstalling(true);
    setError(null);
    try {
      // Başarılıysa uygulama yeniden başlar; buraya dönülmez.
      await api.installUpdate();
    } catch (e) {
      setError(friendlyError(e));
      setInstalling(false);
    }
  }

  const Icon = failed ? TriangleAlert : downloading ? LoaderCircle : Download;
  return (
    <div className="rounded-lg bg-primary/12 px-2.5 py-2 text-xs" role="status" aria-live="polite">
      <div className="flex items-start gap-2.5">
        <Icon className={`mt-px size-4 shrink-0 text-primary ${downloading ? "animate-spin" : ""}`} />
        <div className="min-w-0 flex-1">
          <div className="font-medium">
            {failed ? "Güncelleme indirilemedi" : downloading ? "Güncelleme indiriliyor…" : "Güncelleme hazır"}
          </div>
          <div className="text-muted-foreground">Kum {status.available}</div>
        </div>
      </div>
      {error && <p className="mt-1.5 text-destructive selectable">{error}</p>}
      {status.ready && (
        <Button size="sm" className="mt-2 h-7 w-full" disabled={installing} onClick={install}>
          {installing ? "Kuruluyor…" : "Yükle ve yeniden başlat"}
        </Button>
      )}
      {failed && (
        <Button
          size="sm"
          variant="outline"
          className="mt-2 h-7 w-full bg-background/70"
          onClick={() => api.checkUpdate().then(onStatus, (e) => setError(friendlyError(e)))}
        >
          Tekrar dene
        </Button>
      )}
    </div>
  );
}
