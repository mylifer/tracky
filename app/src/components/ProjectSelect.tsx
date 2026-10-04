import { useMemo, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import type { Client, Tag } from "../api";
import { FILTER_MIN, projectGroups, readRecent, rememberRecent } from "../lib/projectOptions";
import { tagColor } from "../lib/tags";
import { useTaxonomy } from "../lib/taxonomy";
import { cn } from "../lib/utils";

/**
 * Proje seçici: işletim sisteminin açılır listesi (katmanda açılan liste Windows'ta seçimi
 * kaybedebiliyordu). Boş değer `placeholder`'ı gösterir; `extra` listenin sonuna eklenir
 * ("Projesiz", "Zaman çizelgesine alma" gibi).
 * Projeler müşteriye göre gruplanır, son seçilenler en üstte durur; proje çoksa yanında bir
 * süzme alanı çıkar (liste yine yerel kalır, Enter ilk eşleşeni seçer). Müşteri bilgisi
 * verilmezse paylaşılan önbellekten alınır.
 */
export function ProjectSelect({
  value,
  onChange,
  projects,
  placeholder = "Proje seç…",
  extra,
  clients,
  projectClients,
  className,
  "aria-label": ariaLabel = "Proje",
}: {
  value: string;
  onChange: (id: string) => void;
  projects: Tag[];
  /** `null`: ipucu seçeneği yok (değer hep doluysa). */
  placeholder?: string | null;
  extra?: { value: string; label: string }[];
  /** Gruplama için müşteriler; verilmezse sınıflandırmadan okunur. */
  clients?: Client[];
  /** Proje → müşteri; `clients` ile birlikte verilir. */
  projectClients?: Record<string, string>;
  className?: string;
  "aria-label"?: string;
}) {
  const taxonomy = useTaxonomy(!clients);
  const [filter, setFilter] = useState("");
  // Son kullanılanlar açılışta okunur; seçim yapılınca bir sonraki seçici günceli görür.
  const [recent, setRecent] = useState(readRecent);
  const filterable = projects.length >= FILTER_MIN;
  const [groups, matches] = useMemo(() => {
    const opts = {
      clients: clients ?? taxonomy?.clients,
      projectClients: clients ? projectClients : taxonomy?.projectClients,
      recent,
      filter: filterable ? filter : "",
    };
    // Enter'ın seçeceği ilk eşleşen: seçili proje yalnızca süzmeye uyuyorsa sayılır.
    const found = opts.filter ? projectGroups(projects, opts).flatMap((g) => g.projects) : [];
    return [projectGroups(projects, { ...opts, keep: value }), found] as const;
  }, [projects, clients, projectClients, taxonomy, recent, filter, filterable, value]);
  const tag = projects.find((p) => p.id === value);

  function choose(id: string) {
    if (projects.some((p) => p.id === id)) {
      rememberRecent(id);
      setRecent(readRecent());
    }
    setFilter("");
    onChange(id);
  }

  return (
    <span className={cn("flex items-center gap-1", className)}>
      {filterable && (
        <span className="relative flex w-[38%] max-w-28 min-w-14 shrink-0 items-center">
          <Search className="pointer-events-none absolute left-2 size-3 text-muted-foreground" aria-hidden />
          <input
            type="search"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                const first = matches[0];
                if (first) choose(first.id);
              } else if (e.key === "Escape" && filter) {
                // Açık bir menüyü kapatmadan önce süzmeyi temizle.
                e.stopPropagation();
                setFilter("");
              }
            }}
            placeholder="Süz…"
            aria-label={`${ariaLabel}: projeleri süz`}
            title="Proje ya da müşteri adı; Enter ilk eşleşeni seçer"
            className={cn(
              "h-8 w-full min-w-0 rounded-md border bg-background pr-1.5 pl-6 text-xs shadow-xs outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-input/30 [&::-webkit-search-cancel-button]:hidden",
              filter && matches.length === 0 && "border-destructive/60",
            )}
          />
        </span>
      )}
      <span className="relative flex min-w-0 flex-1 items-center">
        <i
          className="pointer-events-none absolute left-2.5 size-2 rounded-full"
          style={{ background: tagColor(tag) }}
          aria-hidden
        />
        <select
          value={value}
          aria-label={ariaLabel}
          onChange={(e) => choose(e.target.value)}
          className="h-8 w-full min-w-0 appearance-none truncate rounded-md border bg-background pr-7 pl-6 text-xs shadow-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-input/30"
        >
          {(placeholder !== null || value === "") && (
            <option value="" disabled>
              {filter && matches.length === 0 ? "Eşleşen proje yok" : (placeholder ?? "")}
            </option>
          )}
          {groups.map((g, i) =>
            g.label ? (
              <optgroup key={`${i}-${g.label}`} label={g.label}>
                {g.projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </optgroup>
            ) : (
              g.projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))
            ),
          )}
          {extra?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
      </span>
    </span>
  );
}
