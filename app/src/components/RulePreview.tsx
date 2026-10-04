import { useEffect, useState } from "react";
import { ArrowRightLeft, Info, Loader2, Sparkles } from "lucide-react";
import { api, formatDuration, type RuleField, type RulePreview as Preview, type Tag } from "../api";
import { cn } from "../lib/utils";

/** Yazmayı bitirince önizle: her tuşta değil. */
const DEBOUNCE_MS = 350;

/**
 * Kural eklenmeden önce etkisi: son 30 günde ne kadar süre bu etikete geçer, ne kadarı başka
 * bir etiketten alınır, hangi pencereler etkilenir.
 */
export function RulePreview({
  tagId,
  field,
  pattern,
  tags,
  className,
}: {
  tagId: string;
  field: RuleField;
  pattern: string;
  /** Başka etiketten alınan sürenin adını göstermek için. */
  tags: Tag[];
  className?: string;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [loading, setLoading] = useState(false);
  const trimmed = pattern.trim();

  useEffect(() => {
    if (!trimmed) {
      setPreview(null);
      return;
    }
    let live = true;
    setLoading(true);
    const id = setTimeout(() => {
      api.previewRule(tagId, field, trimmed).then(
        (p) => live && (setPreview(p), setLoading(false)),
        () => live && (setPreview(null), setLoading(false)),
      );
    }, DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(id);
    };
  }, [tagId, field, trimmed]);

  if (!trimmed) return null;
  const name = (id: string) => tags.find((t) => t.id === id)?.name ?? "başka bir etiket";
  const changes = preview ? preview.gainedSeconds + preview.takenSeconds : 0;

  return (
    <div
      className={cn(
        "rounded-lg border bg-gradient-to-br from-primary/[0.06] to-transparent px-3 py-2.5 text-xs",
        className,
      )}
      aria-live="polite"
    >
      {!preview ? (
        <div className="flex items-center gap-2 text-muted-foreground">
          <Loader2 className="size-3.5 animate-spin" /> Son 30 gün hesaplanıyor…
        </div>
      ) : preview.matchedSeconds === 0 ? (
        <div className="flex items-center gap-2 text-muted-foreground">
          <Info className="size-3.5 shrink-0" /> Son 30 günde bu kurala uyan süre yok; bundan sonrakiler sayılır.
        </div>
      ) : (
        <div className={cn("space-y-1.5 transition-opacity", loading && "opacity-60")}>
          <div className="flex items-start gap-2">
            <Sparkles className="mt-px size-3.5 shrink-0 text-primary" />
            <p>
              {changes > 0 ? (
                <>
                  Son 30 günde <b className="font-semibold tabular">{formatDuration(changes)}</b> bu kurala geçer
                </>
              ) : (
                <>Uyan süre zaten bu kurala ya da elle atanmış; değişen olmaz</>
              )}
              {preview.alreadySeconds > 0 && changes > 0 && (
                <span className="text-muted-foreground"> · {formatDuration(preview.alreadySeconds)} zaten burada</span>
              )}
              .
            </p>
          </div>
          {preview.takenFrom.length > 0 && (
            <div className="flex items-start gap-2 text-amber-700 dark:text-amber-400">
              <ArrowRightLeft className="mt-px size-3.5 shrink-0" />
              <p>
                {preview.takenFrom
                  .slice(0, 3)
                  .map((t) => `${formatDuration(t.seconds)} ${name(t.id)}`)
                  .join(", ")}{" "}
                yerine buraya sayılır.
              </p>
            </div>
          )}
          {preview.blockedSeconds > 0 && (
            <p className="pl-5.5 text-muted-foreground">
              {formatDuration(preview.blockedSeconds)} elle atandığı ya da önce gelen bir kurala uyduğu için değişmez.
            </p>
          )}
          {preview.samples.length > 0 && (
            <ul className="space-y-0.5 pt-0.5 pl-5.5">
              {preview.samples.slice(0, 4).map((s) => (
                <li key={`${s.appName}\u0000${s.title}`} className="flex items-center gap-2 text-muted-foreground">
                  <span className="min-w-0 flex-1 truncate" title={s.title}>
                    <span className="text-foreground/80">{s.appName}</span>
                    {s.title && ` · ${s.title}`}
                  </span>
                  <span className="shrink-0 tabular">{formatDuration(s.seconds)}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
