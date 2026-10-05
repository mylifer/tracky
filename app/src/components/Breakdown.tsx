import { useEffect, useMemo, useState } from "react";
import { ChevronRight, PenLine } from "lucide-react";
import { api, formatDuration, type AppBucket, type Tag, type UsageTotal } from "../api";
import { detailRows, onlyOther } from "../lib/minorWindows";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { AppIcon } from "./AppIcon";
import { CategorySelect } from "./CategorySelect";

const MAX_TITLES = 15;

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
  const max = apps[0]?.seconds || 1;
  // Kısa pencereler adıyla görünmez, "Diğer"de toplanır; tek satır "Diğer" kalırsa liste boş.
  const titleRows = useMemo(() => {
    const rows = detailRows(titles, (t) => t.seconds, undefined, "pencere", MAX_TITLES);
    return onlyOther(rows) ? [] : rows;
  }, [titles]);

  // Açık uygulamanın süresi dakika olarak değişince yenile (her canlı raporda değil).
  const openMinutes = Math.floor((apps.find((a) => a.appId === open)?.seconds ?? 0) / 60);
  useEffect(() => {
    setTitles([]);
    if (!open) return;
    let current = true;
    api.appTitlesBetween(open, start, days).then(
      (t) => current && setTitles(t),
      () => {},
    );
    return () => {
      current = false;
    };
  }, [open, start, days, openMinutes]);

  async function assign(appId: string, tagId: string | null) {
    await api.assignAppCategory(appId, tagId).catch(() => {});
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
            <div
              className={cn(
                "group flex items-center gap-3 rounded-lg px-2 py-1.5 transition-colors hover:bg-accent/60",
                isOpen && "bg-accent/60",
              )}
            >
              <button
                className="flex min-w-0 flex-1 items-center gap-3 text-left"
                onClick={() => setOpen(isOpen ? null : a.appId)}
                aria-expanded={isOpen}
                title={a.appId}
              >
                <ChevronRight
                  className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", isOpen && "rotate-90")}
                />
                <AppIcon
                  appId={a.appId}
                  name={a.appName}
                  size={28}
                  fallback={
                    <span
                      className="grid size-7 shrink-0 place-items-center rounded-[7px] text-xs font-semibold"
                      style={{ background: `color-mix(in srgb, ${color} 18%, transparent)`, color }}
                    >
                      {a.appId.startsWith("kum.manual/") ? <PenLine className="size-3.5" /> : initial(a.appName)}
                    </span>
                  }
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium">{a.appName}</span>
                  <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                    <span
                      className="block h-full rounded-full"
                      style={{ width: `${Math.max(2, (a.seconds / max) * 100)}%`, background: color }}
                    />
                  </span>
                </span>
              </button>
              <CategorySelect
                value={a.categoryId}
                onChange={(id) => assign(a.appId, id)}
                categories={categories}
                noneLabel={UNCATEGORIZED}
                align="end"
                className="w-44 shrink-0"
                aria-label={`${a.appName} kategorisi`}
              />
              <span className="w-[72px] shrink-0 text-right text-[13px] tabular">{formatDuration(a.seconds)}</span>
            </div>
            {isOpen && (
              <ul className="mb-1 ml-[52px] border-l pl-3">
                {titleRows.map((r) =>
                  r.kind === "item" ? (
                    <li key={r.item.key} className="flex items-center gap-3 py-1 pr-2 text-xs">
                      <span className="min-w-0 flex-1 truncate selectable" title={r.item.label}>
                        {r.item.label || <em className="text-muted-foreground">(başlık okunamadı)</em>}
                      </span>
                      <span className="shrink-0 text-muted-foreground tabular">{formatDuration(r.seconds)}</span>
                    </li>
                  ) : (
                    <li key={r.key} className="flex items-center gap-3 py-1 pr-2 text-xs text-muted-foreground">
                      <span className="min-w-0 flex-1 truncate">{r.label}</span>
                      <span className="shrink-0 tabular">{formatDuration(r.seconds)}</span>
                    </li>
                  ),
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
          <li
            key={id ?? "none"}
            className="flex items-center gap-1.5 text-[11px] whitespace-nowrap text-muted-foreground"
          >
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
