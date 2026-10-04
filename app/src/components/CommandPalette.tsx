import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CornerDownLeft, Search } from "lucide-react";
import { fold } from "../lib/search";
import { cn } from "../lib/utils";

export type Command = {
  id: string;
  label: string;
  group: string;
  icon?: ReactNode;
  /** Sağda kısayol ya da ek bilgi. */
  hint?: string;
  /** Aramada etiketin yanında bakılan sözcükler. */
  keywords?: string;
  run: () => void;
};

/** Sorgunun harfleri sırayla geçiyor mu? Sözcük başı ve bitişik eşleşme daha yüksek puan. */
function score(text: string, query: string): number {
  if (!query) return 1;
  const i = text.indexOf(query);
  if (i === 0) return 100;
  if (i > 0) return text[i - 1] === " " ? 80 : 60;
  let t = 0;
  let points = 0;
  for (const ch of query) {
    const at = text.indexOf(ch, t);
    if (at < 0) return 0;
    points += at === t ? 3 : 1;
    t = at + 1;
  }
  return points;
}

/**
 * ⌘K komut paleti: sayfalar, eylemler ve projeler tek kutudan; yazınca süzülür. Eşleşme
 * yoksa (ya da en sonda) yazılanı aramak için bir satır çıkar.
 */
export function CommandPalette({
  open,
  onClose,
  commands,
  onSearch,
}: {
  open: boolean;
  onClose: () => void;
  commands: Command[];
  /** Yazılanı pencere başlıklarında ara. */
  onSearch: (query: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      // Açılış animasyonu başlarken odak kaybolmasın.
      requestAnimationFrame(() => input.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => {
    const q = fold(query.trim());
    const found = commands
      .map((c) => ({ c, s: Math.max(score(fold(c.label), q), score(fold(c.keywords ?? ""), q) * 0.8) }))
      .filter((r) => r.s > 0)
      .sort((a, b) => (q ? b.s - a.s : 0))
      .map((r) => r.c);
    if (query.trim()) {
      found.push({
        id: "search",
        label: `“${query.trim()}” için pencerelerde ara`,
        group: "Ara",
        icon: <Search />,
        run: () => onSearch(query.trim()),
      });
    }
    return found;
  }, [commands, query, onSearch]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (!open) return null;

  function run(c: Command) {
    onClose();
    c.run();
  }

  // Sorgu boşken gruplanmış, yazınca puana göre tek liste.
  const grouped = !query.trim();
  let lastGroup = "";

  return (
    <div className="fixed inset-0 z-[90]" role="dialog" aria-label="Komut paleti" aria-modal>
      <div
        className="absolute inset-0 animate-in bg-black/25 backdrop-blur-[2px] duration-150 fade-in-0 dark:bg-black/45"
        onPointerDown={onClose}
      />
      <div className="absolute inset-x-0 top-[14%] mx-auto w-[min(560px,calc(100%-2rem))] animate-in overflow-hidden rounded-2xl border bg-popover/95 text-popover-foreground shadow-2xl backdrop-blur-xl duration-150 fade-in-0 zoom-in-[0.98] slide-in-from-top-2">
        <div className="flex items-center gap-3 border-b px-4">
          <Search className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={input}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Sayfa, eylem ya da proje ara…"
            className="h-12 min-w-0 flex-1 bg-transparent text-[14px] outline-none placeholder:text-muted-foreground"
            aria-label="Komut ara"
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((a) => Math.min(results.length - 1, a + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => Math.max(0, a - 1));
              } else if (e.key === "Enter") {
                e.preventDefault();
                const c = results[active];
                if (c) run(c);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              }
            }}
          />
          <kbd className="rounded border bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">esc</kbd>
        </div>
        <div ref={list} className="max-h-[min(420px,60vh)] overflow-y-auto p-1.5" role="listbox">
          {results.length === 0 && <p className="px-3 py-6 text-center text-xs text-muted-foreground">Sonuç yok</p>}
          {results.map((c, i) => {
            const header = grouped && c.group !== lastGroup;
            lastGroup = c.group;
            return (
              <div key={c.id}>
                {header && (
                  <div className="px-2.5 pt-2.5 pb-1 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">
                    {c.group}
                  </div>
                )}
                <button
                  data-index={i}
                  role="option"
                  aria-selected={i === active}
                  onMouseMove={() => setActive(i)}
                  onClick={() => run(c)}
                  className={cn(
                    "flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] [&_svg]:size-4 [&_svg]:shrink-0",
                    i === active ? "bg-primary text-primary-foreground" : "text-foreground",
                  )}
                >
                  <span className={cn(i === active ? "text-primary-foreground" : "text-muted-foreground")}>
                    {c.icon}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{c.label}</span>
                  {!grouped && (
                    <span
                      className={cn(
                        "text-[11px]",
                        i === active ? "text-primary-foreground/70" : "text-muted-foreground",
                      )}
                    >
                      {c.group}
                    </span>
                  )}
                  {c.hint && (
                    <kbd
                      className={cn(
                        "rounded px-1.5 py-0.5 text-[10px] font-medium",
                        i === active ? "bg-white/20 text-primary-foreground" : "bg-muted text-muted-foreground",
                      )}
                    >
                      {c.hint}
                    </kbd>
                  )}
                  {i === active && <CornerDownLeft className="size-3.5! opacity-70" />}
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
