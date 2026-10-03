import { useEffect, useState, type ReactNode } from "react";
import { CheckCircle2, Circle } from "lucide-react";
import { api, type AppStatus } from "./api";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Switch } from "./components/ui/switch";
import { applyPlatform } from "./lib/theme";
import { cn } from "./lib/utils";

type Props = { status: AppStatus; onChange: () => void };

export default function Onboarding({ status, onChange }: Props) {
  const [accessibility, setAccessibility] = useState(status.accessibility);
  const [autostart, setAutostart] = useState(status.autostart);
  const isMac = status.platform === "macos";

  // Karşılama ekranı tek parça yüzey: yerel malzeme yerine düz zemin.
  useEffect(() => applyPlatform(status.platform, "none"), [status.platform]);

  // İzin Sistem Ayarları'nda verilir; geri dönüldüğünde fark etmek için yokla.
  useEffect(() => {
    if (accessibility) return;
    const id = setInterval(async () => {
      const s = await api.status();
      if (s.accessibility) setAccessibility(true);
    }, 1500);
    return () => clearInterval(id);
  }, [accessibility]);

  async function toggleAutostart(next: boolean) {
    await api.setAutostart(next);
    setAutostart(next);
  }

  async function finish() {
    await api.completeOnboarding();
    applyPlatform(status.platform, status.effect);
    onChange();
  }

  return (
    <main data-tauri-drag-region className="grid h-full place-items-center bg-muted/40 p-6">
      <div className="w-full max-w-md space-y-6 rounded-2xl border bg-card p-7 shadow-xl">
        <div className="space-y-3 text-center">
          <img src="/icon.png" alt="" className="mx-auto size-14" />
          <h1 className="text-lg font-semibold">Kum'a hoş geldin</h1>
          <p className="text-[13px] text-muted-foreground">
            Kum, hangi uygulamada ve pencerede ne kadar zaman geçirdiğini sessizce kaydeder. Veriler yalnızca bu
            bilgisayarda tutulur.
          </p>
        </div>

        <div className="divide-y rounded-xl border">
          <Step done={accessibility} title="Pencere başlıklarını okuma izni">
            <p>
              {isMac
                ? "Hangi pencerede olduğunu görmek için Erişilebilirlik izni gerekir. Sistem Ayarları'nda Kum'u etkinleştir."
                : "Bu sistemde ek izin gerekmez."}
            </p>
            {accessibility ? (
              <Badge variant="success" className="mt-2">
                Verildi
              </Badge>
            ) : (
              <div className="mt-2.5 flex gap-2">
                <Button size="sm" onClick={() => api.requestAccessibility()}>
                  İzin İste
                </Button>
                <Button size="sm" variant="outline" onClick={() => api.openAccessibilitySettings()}>
                  Ayarları Aç
                </Button>
              </div>
            )}
          </Step>
          <Step
            done
            title="Bilgisayar açılınca başlat"
            action={<Switch checked={autostart} onCheckedChange={toggleAutostart} />}
          >
            <p>Takip arka planda, menü çubuğunda devam eder.</p>
          </Step>
        </div>

        <Button size="lg" className="w-full" disabled={!accessibility} onClick={finish}>
          {accessibility ? "Başla" : "İzin bekleniyor…"}
        </Button>
      </div>
    </main>
  );
}

function Step({
  done,
  title,
  action,
  children,
}: {
  done: boolean;
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  const Icon = done ? CheckCircle2 : Circle;
  return (
    <div className="flex gap-3 p-4">
      <Icon className={cn("mt-px size-4 shrink-0", done ? "text-success" : "text-muted-foreground")} />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">{title}</div>
        <div className="mt-0.5 text-xs text-muted-foreground">{children}</div>
      </div>
      {action}
    </div>
  );
}
