import { useEffect, useState, type ReactNode } from "react";
import { CheckCircle2, Circle, FileSpreadsheet, Plus, X } from "lucide-react";
import { api, type AppStatus } from "./api";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Switch } from "./components/ui/switch";
import { friendlyError, toast } from "./lib/feedback";
import { applyPlatform } from "./lib/theme";
import { nextColor } from "./lib/tags";
import { cn } from "./lib/utils";

type Props = { status: AppStatus; onChange: () => void };

/**
 * Karşılama: önce izinler, sonra (ilk kurulumda) üzerinde çalışılan projeler. Projeler
 * yazılarak ya da firmanın zaman çizelgesi şablonundan eklenir; ilk günden rapor anlamlı olsun.
 */
export default function Onboarding({ status, onChange }: Props) {
  const [accessibility, setAccessibility] = useState(status.accessibility);
  const [autostart, setAutostart] = useState(status.autostart);
  const [step, setStep] = useState<"permissions" | "projects">("permissions");
  const isMac = status.platform === "macos";

  // Karşılama ekranı tek parça yüzey: yerel malzeme yerine düz zemin.
  useEffect(() => applyPlatform(status.platform, "none"), [status.platform]);

  // İzin Sistem Ayarları'nda verilir; geri dönüldüğünde fark etmek için yokla.
  useEffect(() => {
    if (accessibility) return;
    const id = setInterval(async () => {
      try {
        const s = await api.status();
        if (s.accessibility) setAccessibility(true);
      } catch {
        // Bir yoklama okunamadıysa sonraki dener.
      }
    }, 1500);
    return () => clearInterval(id);
  }, [accessibility]);

  async function toggleAutostart(next: boolean) {
    try {
      await api.setAutostart(next);
      setAutostart(next);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  async function finish() {
    await api.completeOnboarding();
    applyPlatform(status.platform, status.effect);
    onChange();
  }

  return (
    <main data-tauri-drag-region className="relative grid h-full place-items-center overflow-hidden bg-background p-6">
      {/* Arka planda yumuşak kum ışıkları. */}
      <div
        aria-hidden
        className="pointer-events-none absolute -top-40 -left-32 size-[520px] rounded-full bg-[radial-gradient(circle,var(--brand-1),transparent_65%)] opacity-25 blur-2xl"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -right-40 -bottom-48 size-[560px] rounded-full bg-[radial-gradient(circle,var(--brand-2),transparent_65%)] opacity-20 blur-2xl"
      />
      <div className="relative w-full max-w-md animate-in space-y-6 rounded-3xl border bg-card/85 p-7 shadow-[var(--shadow-raised)] backdrop-blur-xl duration-300 fade-in-0 zoom-in-[0.98]">
        <div className="space-y-3 text-center">
          <img src="/icon.png" alt="" className="mx-auto size-16 drop-shadow-lg" />
          <h1 className="text-xl font-semibold tracking-tight">
            {step === "permissions" ? (
              <>
                <span className="text-brand">Kum</span>'a hoş geldin
              </>
            ) : (
              "Neler üzerinde çalışıyorsun?"
            )}
          </h1>
          <p className="text-[13px] text-muted-foreground">
            {step === "permissions"
              ? "Kum, hangi uygulamada ve pencerede ne kadar zaman geçirdiğini sessizce kaydeder. Veriler yalnızca bu bilgisayarda tutulur."
              : "Projelerini ekle; pencere başlığında adı geçen her şey o projeye yazılır. Sonradan da ekleyebilirsin."}
          </p>
          {!status.onboarded && (
            <div className="flex justify-center gap-1.5" aria-hidden>
              {(["permissions", "projects"] as const).map((s) => (
                <span
                  key={s}
                  className={cn(
                    "h-1.5 rounded-full transition-all duration-300",
                    step === s ? "w-6 bg-brand" : "w-1.5 bg-muted-foreground/30",
                  )}
                />
              ))}
            </div>
          )}
        </div>

        {step === "permissions" ? (
          <>
            <div className="divide-y rounded-xl border bg-background/50">
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

            <Button
              size="lg"
              className="w-full"
              disabled={!accessibility}
              onClick={() =>
                status.onboarded
                  ? finish().catch((e) => toast(friendlyError(e), { tone: "error" }))
                  : setStep("projects")
              }
            >
              {!accessibility ? "İzin bekleniyor…" : status.onboarded ? "Başla" : "Devam"}
            </Button>
          </>
        ) : (
          <ProjectsStep onDone={finish} onBack={() => setStep("permissions")} />
        )}
      </div>
    </main>
  );
}

/** Proje adları yazılır (Enter ile eklenir) ya da zaman çizelgesi şablonundan alınır. */
function ProjectsStep({ onDone, onBack }: { onDone: () => Promise<void>; onBack: () => void }) {
  const [names, setNames] = useState<string[]>([]);
  const [word, setWord] = useState("");
  const [imported, setImported] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function add() {
    const n = word.trim();
    if (n && !names.some((x) => x.toLocaleLowerCase("tr") === n.toLocaleLowerCase("tr"))) setNames([...names, n]);
    setWord("");
  }

  async function importTemplate() {
    setError(null);
    try {
      const path = await api.pickTimesheetFile();
      if (!path) return;
      const r = await api.importTimesheetTemplate(null, path);
      setImported(
        r.created.length
          ? `Şablondan ${r.created.length} proje eklendi: ${r.created.join(", ")}.`
          : "Şablon bağlandı; yeni proje bulunmadı.",
      );
    } catch (e) {
      setError(friendlyError(e));
    }
  }

  async function finish() {
    setBusy(true);
    setError(null);
    try {
      const pending = [...names];
      if (word.trim()) pending.push(word.trim());
      const { tags, rules } = await api.taxonomy();
      const all = [...tags];
      for (const name of pending) {
        const existing = all.find(
          (t) => t.kind === "project" && t.name.toLocaleLowerCase("tr") === name.toLocaleLowerCase("tr"),
        );
        // Önceki denemede proje eklenip kuralı eklenemediyse kural şimdi eklenir.
        if (existing && rules.some((r) => r.tagId === existing.id)) continue;
        const tag = existing ?? (await api.saveTag({ kind: "project", name, color: nextColor(all) }));
        if (!existing) all.push(tag);
        // Kuralsız proje süre toplamaz: adı başlıkta aranan sözcük olur.
        await api.addRule(tag.id, "title", name);
      }
      await onDone();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2.5 rounded-xl border bg-background/50 p-3">
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <Input
            autoFocus
            value={word}
            onChange={(e) => setWord(e.target.value)}
            placeholder="Proje adı, örn. Togg Loyalty"
            aria-label="Proje adı"
          />
          <Button type="submit" variant="outline" disabled={!word.trim()}>
            <Plus /> Ekle
          </Button>
        </form>
        {names.length > 0 && (
          <ul className="flex flex-wrap gap-1.5">
            {names.map((n, i) => (
              <li
                key={n}
                className="inline-flex animate-in items-center gap-1.5 rounded-full border bg-card py-0.5 pr-0.5 pl-2.5 text-xs duration-200 fade-in-0 zoom-in-95"
              >
                <i className="size-2 rounded-full" style={{ background: `var(--c${(i % 8) + 1})` }} />
                {n}
                <button
                  type="button"
                  className="grid size-5 place-items-center rounded-full text-muted-foreground hover:bg-accent hover:text-foreground"
                  onClick={() => setNames(names.filter((x) => x !== n))}
                  aria-label={`${n} kaldır`}
                >
                  <X className="size-3" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <button
          type="button"
          onClick={importTemplate}
          className="flex w-full items-center gap-2.5 rounded-lg border border-dashed px-3 py-2 text-left text-xs text-muted-foreground hover:border-primary/40 hover:bg-accent/40 hover:text-foreground"
        >
          <FileSpreadsheet className="size-4 shrink-0 text-emerald-600" />
          <span>
            <span className="font-medium text-foreground">Zaman çizelgesi şablonundan al</span>
            <br />
            Firmanın Excel dosyasındaki projeler ve birimler eklenir.
          </span>
        </button>
        {imported && <p className="text-xs text-success">{imported}</p>}
      </div>
      {error && <p className="text-xs text-destructive selectable">{error}</p>}
      <div className="flex gap-2">
        <Button variant="ghost" onClick={onBack}>
          Geri
        </Button>
        <Button size="lg" className="flex-1" onClick={finish} disabled={busy}>
          {names.length || word.trim() ? "Projeleri ekle ve başla" : "Şimdilik geç ve başla"}
        </Button>
      </div>
    </div>
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
