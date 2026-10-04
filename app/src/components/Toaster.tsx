import { useEffect, useState } from "react";
import { CheckCircle2, CircleAlert, Undo2, X } from "lucide-react";
import { dismissToast, undo, useToasts, type Toast } from "../lib/feedback";
import { cn } from "../lib/utils";

/** Bildirimin ekranda kalma süresi; geri alınabilenler daha uzun kalır. */
const LIFETIME = { plain: 4500, undo: 8000 };

/** Pencerenin altında, ortada: kısa bildirimler ve "Geri al". */
export function Toaster() {
  const toasts = useToasts();
  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed inset-x-0 bottom-5 z-[100] flex flex-col items-center gap-2 px-4"
    >
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}

function ToastItem({ toast }: { toast: Toast }) {
  const [hover, setHover] = useState(false);
  useEffect(() => {
    if (hover) return;
    const id = setTimeout(() => dismissToast(toast.id), toast.undo ? LIFETIME.undo : LIFETIME.plain);
    return () => clearTimeout(id);
  }, [hover, toast]);
  const Icon = toast.tone === "error" ? CircleAlert : toast.tone === "success" ? CheckCircle2 : null;
  return (
    <div
      role={toast.tone === "error" ? "alert" : "status"}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      className={cn(
        "pointer-events-auto flex max-w-[min(32rem,100%)] items-center gap-2.5 rounded-xl border py-2 pr-2 pl-3.5 text-[13px] shadow-lg backdrop-blur-xl",
        "animate-in duration-200 fade-in-0 slide-in-from-bottom-3",
        "border-white/10 bg-neutral-900/92 text-white dark:border-white/12 dark:bg-neutral-800/95",
      )}
    >
      {Icon && <Icon className={cn("size-4 shrink-0", toast.tone === "error" ? "text-red-400" : "text-emerald-400")} />}
      <span className="min-w-0 flex-1 selectable">{toast.message}</span>
      {toast.undo !== undefined && (
        <button
          className="flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-xs font-semibold text-sky-300 hover:bg-white/10"
          onClick={() => {
            dismissToast(toast.id);
            undo(toast.undo!);
          }}
        >
          <Undo2 className="size-3.5" /> Geri al
        </button>
      )}
      {toast.action && (
        <button
          className="h-7 shrink-0 rounded-md px-2 text-xs font-semibold text-sky-300 hover:bg-white/10"
          onClick={() => {
            dismissToast(toast.id);
            toast.action!.run();
          }}
        >
          {toast.action.label}
        </button>
      )}
      <button
        className="grid size-7 shrink-0 place-items-center rounded-md text-white/50 hover:bg-white/10 hover:text-white"
        onClick={() => dismissToast(toast.id)}
        aria-label="Kapat"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
