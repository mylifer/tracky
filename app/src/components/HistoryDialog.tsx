import { useEffect, useState } from "react";
import { History, Undo2 } from "lucide-react";
import { api, type HistoryItem } from "../api";
import { formatTime } from "../lib/dates";
import { friendlyError, notifyChanged, toast, useChanged } from "../lib/feedback";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./ui/dialog";

/**
 * Değişiklik geçmişi: bu açılışta yapılan ve hâlâ geri alınabilecek atama, silme, birleştirme,
 * kural ve arşiv değişiklikleri; en yeni üstte. Aynı süreye dokunan daha yeni bir değişiklik
 * varsa eskisi ondan önce geri alınamaz (düğmesi kapalı).
 */
export function HistoryDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [items, setItems] = useState<HistoryItem[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const load = () => api.undoHistory().then(setItems, () => setItems([]));
  useEffect(() => {
    if (open) load();
  }, [open]);
  // Başka bir yerde geri alınan ya da yapılan değişiklik listeye yansısın.
  useChanged(() => {
    if (open) load();
  });

  async function undo(id: number) {
    setBusy(id);
    try {
      await api.undo(id);
      toast("Geri alındı", { tone: "success" });
      notifyChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setBusy(null);
      load();
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <div className="flex items-center gap-2">
          <History className="size-4 text-muted-foreground" />
          <DialogTitle>Değişiklik geçmişi</DialogTitle>
        </div>
        <DialogDescription>
          Bu açılışta yapılan ve geri alınabilecek değişiklikler. Aynı süreye dokunan değişiklikler sondan başa geri
          alınır.
        </DialogDescription>
        {items && items.length === 0 && (
          <p className="py-6 text-center text-xs text-muted-foreground">Geri alınacak değişiklik yok.</p>
        )}
        {items && items.length > 0 && (
          <ul className="-mx-1 flex max-h-[min(60vh,28rem)] flex-col overflow-y-auto">
            {items.map((item) => (
              <li key={item.id} className="flex items-center gap-3 rounded-md px-1 py-1.5 hover:bg-accent/50">
                <span className="w-11 shrink-0 text-xs text-muted-foreground tabular-nums">
                  {formatTime(new Date(item.at))}
                </span>
                <span className="min-w-0 flex-1 truncate text-[13px]" title={item.label ?? undefined}>
                  {item.label ?? "Değişiklik"}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={item.blocked || busy !== null}
                  title={item.blocked ? "Önce aynı süreye dokunan daha yeni değişikliği geri al" : undefined}
                  onClick={() => undo(item.id)}
                >
                  <Undo2 />
                  Geri al
                </Button>
              </li>
            ))}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}
