import { useEffect, useState } from "react";
import { Square, Timer } from "lucide-react";
import { api, type FocusState } from "../api";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

const OPTIONS = [25, 50, 90];

function clock(secs: number) {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** Kenar çubuğunda odak zamanlayıcısı: başlat, geri sayım, bitir. */
export default function FocusCard({ focus }: { focus: FocusState | null }) {
  const [now, setNow] = useState(Date.now());
  const [open, setOpen] = useState(false);
  // Durum olayı her geldiğinde nesne yenilenir; sayaç yalnızca zamanlayıcı değişince kurulur.
  const endsAt = focus?.endsAt;
  useEffect(() => {
    if (!endsAt) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [endsAt]);

  if (!focus) {
    return (
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm" className="w-full justify-start gap-2 bg-background/70 dark:bg-white/5">
            <Timer className="size-3.5 text-focus" /> Odak başlat
          </Button>
        </PopoverTrigger>
        <PopoverContent side="top" align="start" className="w-[196px] p-2">
          <p className="px-1.5 pt-0.5 pb-2 text-xs text-muted-foreground">Ne kadar odaklanacaksın?</p>
          <div className="grid grid-cols-3 gap-1.5">
            {OPTIONS.map((m) => (
              <Button
                key={m}
                variant="secondary"
                size="sm"
                onClick={() => {
                  setOpen(false);
                  api.startFocus(m);
                }}
              >
                {m} dk
              </Button>
            ))}
          </div>
        </PopoverContent>
      </Popover>
    );
  }

  const start = +new Date(focus.startedAt);
  const end = +new Date(focus.endsAt);
  const left = Math.max(0, Math.round((end - now) / 1000));
  const progress = Math.min(1, (now - start) / (end - start));
  return (
    <div className="rounded-lg border border-focus/30 bg-focus/10 p-2.5">
      <div className="flex items-center gap-2">
        <Timer className="size-3.5 shrink-0 text-focus" />
        <span className="flex-1 text-xs font-medium">Odak · {focus.minutes} dk</span>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-6 text-muted-foreground"
          onClick={() => api.stopFocus()}
          aria-label="Odağı bitir"
          title="Odağı bitir"
        >
          <Square className="size-3" />
        </Button>
      </div>
      <div className="mt-1 text-[20px] leading-none font-semibold tabular">{clock(left)}</div>
      <div className="mt-2 h-1 overflow-hidden rounded-full bg-focus/15">
        <div className="h-full rounded-full bg-focus transition-[width]" style={{ width: `${progress * 100}%` }} />
      </div>
    </div>
  );
}
