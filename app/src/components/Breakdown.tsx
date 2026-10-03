import { useEffect, useState } from "react";
import { ChevronRight } from "lucide-react";
import { api, formatDuration, type AppBucket, type Tag, type UsageTotal } from "../api";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "./ui/select";

const MAX_TITLES = 15;
const NONE = "__none__";

/** Uygulamalar: harf rozeti (kategori renginde), kategori ataması, tıklayınca pencere başlıkları. */
export function AppList({
  apps,
  tags,
  categories,
  start,
  days,
  onChanged,
}: {
  apps: AppBucket[];
  tags: Map<string, Tag>;
  categories: Tag[];
  start: string;
  days: number;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  const [titles, setTitles] = useState<UsageTotal[]>([]);
  const max = apps[0]?.seconds ?? 1;

  useEffect(() => {
    if (open) api.appTitlesBetween(open, start, days).then(setTitles);
  }, [open, start, days, apps]);

  async function assign(appId: string, tagId: string) {
    await api.assignAppCategory(appId, tagId === NONE ? null : tagId);
    onChanged();
  }

  return (
    <ul>
      {apps.map((a) => {
        const tag = a.categoryId ? tags.get(a.categoryId) : undefined;
        const color = tagColor(tag);
        const isOpen = open === a.appId;
        return (
          <li key={a.appId}>
            <div className={cn("group flex items-center gap-3 rounded-lg px-2 py-1.5 transition-colors hover:bg-accent/60", isOpen && "bg-accent/60")}>
              <button
                className="flex min-w-0 flex-1 items-center gap-3 text-left"
                onClick={() => setOpen(isOpen ? null : a.appId)}
                aria-expanded={isOpen}
                title={a.appId}
              >
                <ChevronRight className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", isOpen && "rotate-90")} />
                <span
                  className="grid size-7 shrink-0 place-items-center rounded-[7px] text-xs font-semibold"
                  style={{ background: `color-mix(in srgb, ${color} 18%, transparent)`, color }}
                >
                  {initial(a.appName)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium">{a.appName}</span>
                  <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                    <span className="block h-full rounded-full" style={{ width: `${Math.max(2, (a.seconds / max) * 100)}%`, background: color }} />
                  </span>
                </span>
              </button>
              <Select value={a.categoryId ?? NONE} onValueChange={(v) => assign(a.appId, v)}>
                <SelectTrigger size="sm" className="w-36 shrink-0" aria-label={`${a.appName} kategorisi`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent align="end">
                  <SelectItem value={NONE}>
                    <Dot color={tagColor(undefined)} />
                    {UNCATEGORIZED}
                  </SelectItem>
                  <SelectSeparator />
                  {categories.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      <Dot color={tagColor(c)} />
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <span className="w-[72px] shrink-0 text-right text-[13px] tabular">{formatDuration(a.seconds)}</span>
            </div>
            {isOpen && (
              <ul className="mb-1 ml-[52px] border-l pl-3">
                {titles.slice(0, MAX_TITLES).map((t) => (
                  <li key={t.key} className="flex items-center gap-3 py-1 pr-2 text-xs">
                    <span className="min-w-0 flex-1 truncate selectable" title={t.label}>
                      {t.label || <em className="text-muted-foreground">(başlık okunamadı)</em>}
                    </span>
                    <span className="shrink-0 text-muted-foreground tabular">{formatDuration(t.seconds)}</span>
                  </li>
                ))}
                {titles.length > MAX_TITLES && (
                  <li className="py-1 text-xs text-muted-foreground">+{titles.length - MAX_TITLES} başlık daha</li>
                )}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/** Grafik efsanesi: görünen kategoriler, sabit sırada. */
export function Legend({ order, tags }: { order: (string | null)[]; tags: Map<string, Tag> }) {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1">
      {order.map((id) => {
        const tag = id ? tags.get(id) : undefined;
        return (
          <li key={id ?? "none"} className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <Dot color={tagColor(tag)} />
            {tag?.name ?? UNCATEGORIZED}
          </li>
        );
      })}
    </ul>
  );
}

export function Dot({ color, className }: { color: string; className?: string }) {
  return <i className={cn("inline-block size-2 shrink-0 rounded-full", className)} style={{ background: color }} />;
}

function initial(name: string): string {
  const ch = [...name.trim()][0] ?? "?";
  return ch.toLocaleUpperCase("tr-TR");
}
