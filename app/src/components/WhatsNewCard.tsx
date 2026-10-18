import { useEffect, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { api, type WhatsNew } from "../api";

/** Notun en çok bu kadar maddesi kartta açık görünür; kalanı "tümü" ile açılır. */
const FIRST_ITEMS = 4;

/**
 * Kenar çubuğunda "Yenilikler": kurulan sürümün notları, ilk açılışta. Kapatılınca bu sürüm
 * için bir daha görünmez.
 */
export function WhatsNewCard() {
  const [news, setNews] = useState<WhatsNew | null>(null);
  const [all, setAll] = useState(false);
  useEffect(() => {
    api.whatsNew().then(setNews, () => {});
  }, []);
  if (!news) return null;

  const items = news.notes
    .split("\n")
    .map((l) => l.replace(/^\s*[-*•]\s*/, "").trim())
    .filter(Boolean);
  const shown = all ? items : items.slice(0, FIRST_ITEMS);
  const close = () => {
    setNews(null);
    api.dismissWhatsNew().catch(() => {});
  };
  return (
    <section
      className="rounded-lg border border-brand-2/25 bg-brand-soft px-2.5 py-2 text-xs"
      aria-label={`Kum ${news.version} yenilikleri`}
    >
      <div className="flex items-center gap-2">
        <Sparkles className="size-4 shrink-0 text-brand-2" />
        <div className="min-w-0 flex-1 font-medium">Kum {news.version} kuruldu</div>
        <button
          onClick={close}
          aria-label="Yenilikleri kapat"
          title="Kapat"
          className="rounded p-0.5 text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <X className="size-3.5" />
        </button>
      </div>
      <ul className="mt-1.5 space-y-1 pl-6 text-foreground/85">
        {shown.map((item) => (
          <li key={item} className="list-disc leading-snug">
            {item}
          </li>
        ))}
      </ul>
      {items.length > FIRST_ITEMS && (
        <button
          onClick={() => setAll(!all)}
          className="mt-1 pl-6 text-[11px] text-brand-ink outline-none dark:text-brand-1 hover:underline focus-visible:underline"
        >
          {all ? "Daha az" : `Tümü (${items.length})`}
        </button>
      )}
    </section>
  );
}
