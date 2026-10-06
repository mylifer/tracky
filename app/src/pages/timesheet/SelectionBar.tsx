import { Combine, Loader2, Sheet, Trash2, X } from "lucide-react";
import { type EntryView } from "../../api";
import { Button } from "../../components/ui/button";
import { blocked, mergeProblem } from "../../lib/timesheet";
import { num } from "./shared";

/** Seçili satırların işlemleri: birleştir, gönder, sil. */
export function SelectionBar({
  rows,
  sendLabel,
  busy,
  merging,
  onClear,
  onMerge,
  onSend,
  onDismiss,
}: {
  rows: EntryView[];
  sendLabel: string;
  busy: boolean;
  /** Birleştirme sürüyor. */
  merging: boolean;
  onClear: () => void;
  onMerge: () => void;
  onSend: () => void;
  onDismiss: () => void;
}) {
  const hours = rows.reduce((s, e) => s + e.hours, 0);
  const mergeWhy = mergeProblem(rows);
  const blockedRows = rows.filter(blocked).length;
  return (
    <div className="sticky bottom-3 z-20 flex justify-center">
      <div
        role="toolbar"
        aria-label="Seçili satırlar"
        className="flex flex-wrap items-center gap-2 rounded-xl border bg-popover px-3 py-2 text-xs shadow-lg"
      >
        <span className="tabular">
          <b className="font-semibold">{rows.length}</b> satır seçildi · {num.format(hours)} sa
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={!!mergeWhy || merging}
          title={mergeWhy ?? "En erken başlangıçta tek satır olur; süreler toplanır, açıklamalar birleşir"}
          onClick={onMerge}
        >
          <Combine /> Birleştir
        </Button>
        <Button
          size="sm"
          disabled={busy || blockedRows > 0}
          title={
            blockedRows > 0
              ? "Seçimde açıklaması boş ya da takipte değişen satır var"
              : "Yalnızca seçili satırları gönder"
          }
          onClick={onSend}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Sheet />} {sendLabel}
        </Button>
        <Button size="sm" variant="ghost" onClick={onDismiss} title="Satırları sil; bildirimden geri alınır">
          <Trash2 /> Sil
        </Button>
        <button
          aria-label="Seçimi kaldır"
          title="Seçimi kaldır (Esc)"
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={onClear}
        >
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
