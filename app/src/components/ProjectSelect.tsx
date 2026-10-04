import { ChevronDown } from "lucide-react";
import type { Tag } from "../api";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

/**
 * Proje seçici: işletim sisteminin açılır listesi (katmanda açılan liste Windows'ta seçimi
 * kaybedebiliyordu). Boş değer `placeholder`'ı gösterir; `extra` listenin sonuna eklenir.
 */
export function ProjectSelect({
  value,
  onChange,
  projects,
  placeholder = "Proje seç…",
  extra,
  className,
  "aria-label": ariaLabel = "Proje",
}: {
  value: string;
  onChange: (id: string) => void;
  projects: Tag[];
  placeholder?: string;
  extra?: { value: string; label: string }[];
  className?: string;
  "aria-label"?: string;
}) {
  const tag = projects.find((p) => p.id === value);
  return (
    <span className={cn("relative flex items-center", className)}>
      <i
        className="pointer-events-none absolute left-2.5 size-2 rounded-full"
        style={{ background: tagColor(tag) }}
        aria-hidden
      />
      <select
        value={value}
        aria-label={ariaLabel}
        onChange={(e) => onChange(e.target.value)}
        className="h-8 w-full appearance-none rounded-md border bg-background pr-7 pl-6 text-xs shadow-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-input/30"
      >
        <option value="" disabled>
          {placeholder}
        </option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
        {extra?.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
    </span>
  );
}
