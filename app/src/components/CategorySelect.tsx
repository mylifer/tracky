import type { ReactNode } from "react";
import type { Tag } from "../api";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./ui/select";

const NONE = "__none__";

/**
 * Kategori seçici: renk noktası ve ad. `noneLabel` verilirse en üstte "kategori yok" seçeneği
 * (değeri `null`) bulunur; verilmezse boş değer `placeholder`'ı gösterir.
 */
export function CategorySelect({
  value,
  onChange,
  categories,
  noneLabel,
  placeholder,
  icon,
  align,
  className,
  "aria-label": ariaLabel,
}: {
  value: string | null;
  onChange: (id: string | null) => void;
  categories: Tag[];
  noneLabel?: string;
  placeholder?: string;
  /** Tetikleyicide değerin önünde (örn. "Limit ekle…" için artı). */
  icon?: ReactNode;
  align?: "start" | "center" | "end";
  className?: string;
  "aria-label"?: string;
}) {
  const current = value ?? (noneLabel ? NONE : "");
  return (
    <Select value={current} onValueChange={(v) => v && onChange(v === NONE ? null : v)}>
      <SelectTrigger
        size="sm"
        className={cn(icon && "[&>[data-slot=select-value]]:flex-1", className)}
        aria-label={ariaLabel}
      >
        {icon}
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent align={align}>
        {noneLabel && (
          <>
            <SelectItem value={NONE}>
              <i className="size-2 rounded-full" style={{ background: tagColor(undefined) }} />
              {noneLabel}
            </SelectItem>
            <SelectSeparator />
          </>
        )}
        {categories.map((c) => (
          <SelectItem key={c.id} value={c.id}>
            <i className="size-2 rounded-full" style={{ background: tagColor(c) }} />
            {c.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
