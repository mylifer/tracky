import { useEffect, useState } from "react";
import type { BudgetUsage } from "../api";
import { budgetRatio, budgetState, formatDays } from "../lib/budget";
import { cn } from "../lib/utils";
import { Input } from "./ui/input";

/** Dolan bütçe kırmızı, %80'i geçen sarı; değilse varlığın kendi rengi. */
function barColor(u: BudgetUsage, color: string) {
  const state = budgetState(u);
  return state === "over" ? "var(--destructive)" : state === "near" ? "var(--color-amber-500, #f59e0b)" : color;
}

/**
 * Bütçe çubuğu: harcanan / anlaşılan adam-gün. `compact` satır içinde kısa çubuk ve yüzde;
 * değilse altta "12,5 / 20 ag · 7,5 ag kaldı".
 */
export function BudgetMeter({
  usage,
  dayHours,
  color,
  compact,
  className,
}: {
  usage: BudgetUsage;
  dayHours: number;
  color: string;
  compact?: boolean;
  className?: string;
}) {
  const ratio = budgetRatio(usage);
  const state = budgetState(usage);
  const used = formatDays(usage.usedSeconds, dayHours);
  const total = formatDays(usage.budgetSeconds, dayHours);
  const left = usage.budgetSeconds - usage.usedSeconds;
  const label = `Bütçe: ${used} / ${total} adam-gün (%${Math.round(ratio * 100)})`;
  const bar = (
    <span className={cn("block h-1 overflow-hidden rounded-full bg-muted", compact ? "w-14" : "w-full")}>
      <span
        className="block h-full rounded-full"
        style={{ width: `${Math.min(100, ratio * 100)}%`, background: barColor(usage, color) }}
      />
    </span>
  );
  if (compact)
    return (
      <span className={cn("flex items-center gap-1.5", className)} title={label} aria-label={label}>
        {bar}
        <span
          className={cn(
            "text-[11px] tabular",
            state === "over"
              ? "text-destructive"
              : state === "near"
                ? "text-amber-600 dark:text-amber-400"
                : "text-muted-foreground",
          )}
        >
          %{Math.round(ratio * 100)}
        </span>
      </span>
    );
  return (
    <span className={cn("block space-y-1", className)} aria-label={label}>
      {bar}
      <span className="flex justify-between gap-2 text-[11px] text-muted-foreground tabular">
        <span>
          <span className="text-foreground">{used}</span> / {total} adam-gün
        </span>
        <span className={cn(state === "over" && "text-destructive")}>
          {left >= 0 ? `${formatDays(left, dayHours)} ag kaldı` : `${formatDays(-left, dayHours)} ag aşıldı`}
        </span>
      </span>
    </span>
  );
}

/** Sözleşme bütçesi girişi (adam-gün); alandan çıkınca kaydedilir, boş bırakmak kaldırır. */
export function BudgetField({
  value,
  onSave,
  label = "Sözleşme bütçesi",
}: {
  value: number | null | undefined;
  onSave: (days: number | null) => void;
  label?: string;
}) {
  const [text, setText] = useState(value ? String(value).replace(".", ",") : "");
  // Başka cihazdan gelen değişiklik eski değerle ezilmesin.
  useEffect(() => setText(value ? String(value).replace(".", ",") : ""), [value]);
  return (
    <span className="inline-flex items-center gap-2">
      <Input
        inputMode="decimal"
        className="h-8 w-24 text-right text-sm tabular"
        placeholder="—"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const t = text.trim();
          const days = t ? Number(t.replace(",", ".")) : null;
          if (days !== null && !(days > 0 && days <= 100000)) {
            setText(value ? String(value).replace(".", ",") : "");
            return;
          }
          if (days !== (value ?? null)) onSave(days);
        }}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        aria-label={`${label} (adam-gün)`}
      />
      <span className="text-xs text-muted-foreground">adam-gün</span>
    </span>
  );
}
