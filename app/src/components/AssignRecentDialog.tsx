import { useEffect, useMemo, useState } from "react";
import { ClockArrowDown } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, formatDuration, type UsageTotal } from "../api";
import { formatTime } from "../lib/dates";
import { friendlyError, notifyChanged, toast, undoable } from "../lib/feedback";
import { useTaxonomy } from "../lib/taxonomy";
import { cn } from "../lib/utils";
import { ProjectSelect } from "./ProjectSelect";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./ui/dialog";

const CHOICES = [15, 30, 60, 120];
const MINUTES_KEY = "kum.assignRecentMinutes";

function savedMinutes(): number {
  try {
    const n = Number(localStorage.getItem(MINUTES_KEY));
    return CHOICES.includes(n) ? n : 30;
  } catch {
    return 30;
  }
}

function label(minutes: number) {
  return minutes < 60 ? `${minutes} dk` : `${minutes / 60} sa`;
}

/**
 * Son süreyi projeye ata: şimdiye kadarki son 15/30/60/120 dakika seçilen projeye yazılır
 * (geri alınabilir, değişiklik geçmişine girer). Bitmiş bir işi sonradan doğru projeye almak
 * için; kısayolla ya da menü çubuğundan açıldıysa (`fromOutside`) atamadan sonra pencere gizlenir
 * ve kullanıcı çalıştığı uygulamaya döner.
 */
export function AssignRecentDialog({
  open,
  fromOutside,
  onClose,
}: {
  open: boolean;
  fromOutside: boolean;
  onClose: () => void;
}) {
  const taxonomy = useTaxonomy(open);
  const projects = useMemo(() => taxonomy?.tags.filter((t) => t.kind === "project") ?? [], [taxonomy]);
  const [minutes, setMinutes] = useState(savedMinutes);
  const [projectId, setProjectId] = useState("");
  const [apps, setApps] = useState<UsageTotal[] | null>(null);
  const [busy, setBusy] = useState(false);
  // Aralığın sonu pencere açıldığı an: önizleme ile atanan aynı aralık olsun.
  const [end, setEnd] = useState(() => new Date());
  const start = useMemo(() => new Date(end.getTime() - minutes * 60_000), [end, minutes]);

  useEffect(() => {
    if (open) {
      setEnd(new Date());
      setProjectId("");
    }
  }, [open]);
  useEffect(() => {
    if (!open) return;
    let live = true;
    setApps(null);
    api.rangeApps(start.toISOString(), end.toISOString()).then(
      (a) => live && setApps(a),
      () => live && setApps([]),
    );
    return () => {
      live = false;
    };
  }, [open, start, end]);

  function choose(m: number) {
    setMinutes(m);
    try {
      localStorage.setItem(MINUTES_KEY, String(m));
    } catch {
      // Tercih hatırlanmasa da olur.
    }
  }

  async function assign() {
    const project = projects.find((p) => p.id === projectId);
    if (!project) return;
    setBusy(true);
    try {
      await undoable(
        api.setRangeProject(start.toISOString(), new Date().toISOString(), project.id),
        `Son ${label(minutes)} → ${project.name}`,
      );
      notifyChanged();
      onClose();
      if (fromOutside) await getCurrentWindow().hide();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(false);
    }
  }

  const tracked = apps?.reduce((sum, a) => sum + a.seconds, 0) ?? 0;
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        className="sm:max-w-md"
        // Asıl seçim proje: klavyeyle hemen seçilip Enter'a basılabilsin.
        onOpenAutoFocus={(e) => {
          e.preventDefault();
          (e.currentTarget as HTMLElement).querySelector("select")?.focus();
        }}
      >
        <div className="flex items-center gap-2">
          <ClockArrowDown className="size-4 text-muted-foreground" />
          <DialogTitle>Son süreyi projeye ata</DialogTitle>
        </div>
        <DialogDescription>
          {formatTime(start)} – {formatTime(end)} arası seçilen projeye yazılır; boşta geçen süre de dahil. Geri
          alınabilir.
        </DialogDescription>
        <div className="flex gap-1.5" role="radiogroup" aria-label="Süre">
          {CHOICES.map((m) => (
            <button
              key={m}
              role="radio"
              aria-checked={m === minutes}
              onClick={() => choose(m)}
              className={cn(
                "flex-1 rounded-md border px-2 py-1.5 text-xs tabular-nums outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50",
                m === minutes && "border-primary bg-primary/10 font-medium text-foreground",
              )}
            >
              {label(m)}
            </button>
          ))}
        </div>
        <div className="min-h-12 rounded-md bg-muted/50 px-3 py-2 text-xs">
          {apps === null ? (
            <span className="text-muted-foreground">Yükleniyor…</span>
          ) : apps.length === 0 ? (
            <span className="text-muted-foreground">
              Bu aralıkta bilgisayarda kayıt yok (boşta ya da duraklatılmış).
            </span>
          ) : (
            <>
              <div className="mb-1 text-muted-foreground">Bilgisayarda {formatDuration(tracked)}:</div>
              <div className="flex flex-wrap gap-x-3 gap-y-0.5">
                {apps.slice(0, 5).map((a) => (
                  <span key={a.key}>
                    {a.label} <span className="text-muted-foreground tabular-nums">{formatDuration(a.seconds)}</span>
                  </span>
                ))}
              </div>
            </>
          )}
        </div>
        <ProjectSelect value={projectId} onChange={setProjectId} projects={projects} className="w-full" />
        <div className="flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={onClose}>
            Vazgeç
          </Button>
          <Button size="sm" disabled={!projectId || busy} onClick={assign}>
            Ata
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
