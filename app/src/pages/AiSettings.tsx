import { useEffect, useState } from "react";
import { Loader2, Sparkles } from "lucide-react";
import { api, type AiStatus } from "../api";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "../components/ui/alert-dialog";
import { ErrorText, SettingBlock, SettingsGroup, ToggleRow } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";
import { AI_SECTION } from "./settingsSections";

/**
 * Yapay zekâyla açıklama yazma: isteğe bağlı, varsayılan kapalı. Kullanıcının kendi Anthropic API
 * anahtarı yalnızca bu cihazın ayarlarında saklanır (eşitlenmez); istek
 * yalnızca zaman çizelgesindeki düğmeyle gider.
 */
export function AiSettings() {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    api.aiSettings().then(setStatus, (e) => setResult({ ok: false, text: friendlyError(e) }));
  }, []);
  if (!status) return result ? <ErrorText>{result.text}</ErrorText> : null;

  const save = async (enabled: boolean, apiKey?: string) => {
    try {
      setStatus(await api.saveAiSettings(enabled, apiKey));
      if (apiKey !== undefined) setKey("");
      setResult(null);
    } catch (e) {
      setResult({ ok: false, text: friendlyError(e) });
    }
  };
  const test = async () => {
    setBusy(true);
    setResult(null);
    try {
      setResult({ ok: true, text: await api.testAi(key.trim() || undefined) });
    } catch (e) {
      setResult({ ok: false, text: friendlyError(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsGroup
      id={AI_SECTION}
      title="Yapay zekâ"
      description="Zaman çizelgesi açıklamalarını Claude'a yazdır. İsteğe bağlı; kendi Anthropic API anahtarınla çalışır."
    >
      <ToggleRow
        label="Yapay zekâyla yaz"
        hint="Gün kartında ve dönem denetiminde “Yapay zekâyla yaz” düğmesi görünür."
        checked={status.enabled}
        onChange={(v) => save(v)}
      />
      <SettingBlock
        label="Anthropic API anahtarı"
        hint={
          status.hasKey
            ? `Kayıtlı (${status.keyHint ?? "…"}). Değiştirmek için yenisini yaz.`
            : "console.anthropic.com → API Keys'ten oluştur. Yalnızca bu cihazda saklanır, eşitlenmez."
        }
      >
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (key.trim()) save(status.enabled, key.trim());
          }}
        >
          <Input
            type="password"
            className="h-8 w-72 text-sm"
            placeholder={status.hasKey ? "••••••••" : "sk-ant-…"}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            aria-label="Anthropic API anahtarı"
          />
          <Button type="submit" size="sm" variant="outline" disabled={!key.trim()}>
            Kaydet
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || (!key.trim() && !status.hasKey)}
            onClick={test}
          >
            {busy ? <Loader2 className="animate-spin" /> : <Sparkles />} Bağlantıyı dene
          </Button>
          {status.hasKey && (
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button type="button" size="sm" variant="ghost">
                  Anahtarı sil
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>API anahtarı silinsin mi?</AlertDialogTitle>
                  <AlertDialogDescription>
                    Yapay zekâyla yazma da kapanır. Yeniden açmak için anahtarı yeniden yazman gerekir.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Vazgeç</AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-white hover:bg-destructive/90"
                    onClick={() => save(false, "")}
                  >
                    Anahtarı sil
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )}
        </form>
        {result && (
          <p className={cn("text-xs selectable", result.ok ? "text-success" : "text-destructive")}>{result.text}</p>
        )}
        <div className="rounded-lg border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <b className="font-medium text-foreground">Gizlilik.</b> Hiçbir şey kendiliğinden gönderilmez. Yalnızca “Yapay
          zekâyla yaz” düğmesine bastığında, yazılacak satırlar için Anthropic'e (api.anthropic.com) şunlar gider: proje
          ve müşteri adı, tür, saat ve başlangıç; o satırın süresindeki pencere başlıkları, iş anahtarları ve site
          adları ya da toplantı konusu; üslup örneği olarak o projelere daha önce yazdığın en çok 10'ar açıklama
          (projede hiç yoksa diğer projelerinden 5). Gün başına bir istek yapılır; ücreti API anahtarının hesabından
          düşer ({status.model}).
        </div>
      </SettingBlock>
    </SettingsGroup>
  );
}
