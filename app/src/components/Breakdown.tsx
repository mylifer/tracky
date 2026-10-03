import { useEffect, useState } from "react";
import { api, formatDuration, type AppBucket, type Bucket, type Tag, type UsageTotal } from "../api";
import { NO_PROJECT, UNCATEGORIZED, tagColor } from "../lib/tags";
import { IconChevron } from "./Icons";

const MAX_TITLES = 15;

function pct(part: number, total: number) {
  return total > 0 ? Math.round((part / total) * 100) : 0;
}

/** Kategoriler: üstte yığılmış oran çubuğu, altında adlı satırlar (renk tek başına taşımaz). */
export function CategoryBreakdown({
  buckets,
  tags,
  total,
  kind = "category",
}: {
  buckets: Bucket[];
  tags: Map<string, Tag>;
  total: number;
  kind?: "category" | "project";
}) {
  const fallback = kind === "category" ? UNCATEGORIZED : NO_PROJECT;
  return (
    <>
      <div className="stackbar" role="presentation">
        {buckets.map((b) => {
          const tag = b.id ? tags.get(b.id) : undefined;
          return (
            <span
              key={b.id ?? "none"}
              style={{ flexGrow: b.seconds, background: tagColor(tag) }}
              title={`${tag?.name ?? fallback}: ${formatDuration(b.seconds)}`}
            />
          );
        })}
      </div>
      <ul className="rows">
        {buckets.map((b) => {
          const tag = b.id ? tags.get(b.id) : undefined;
          return (
            <li key={b.id ?? "none"} className={b.id ? "" : "dim"}>
              <i className="dot-sq" style={{ background: tagColor(tag) }} />
              <span className="row-name">{tag?.name ?? fallback}</span>
              <span className="row-pct">%{pct(b.seconds, total)}</span>
              <span className="row-time">{formatDuration(b.seconds)}</span>
            </li>
          );
        })}
      </ul>
    </>
  );
}

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
    await api.assignAppCategory(appId, tagId || null);
    onChanged();
  }

  return (
    <ul className="apps">
      {apps.map((a) => {
        const tag = a.categoryId ? tags.get(a.categoryId) : undefined;
        const isOpen = open === a.appId;
        return (
          <li key={a.appId} className={isOpen ? "open" : ""}>
            <div className="app-row">
              <button
                className="app-main"
                onClick={() => setOpen(isOpen ? null : a.appId)}
                aria-expanded={isOpen}
                title={a.appId}
              >
                <span className="avatar" style={{ ["--c" as string]: tagColor(tag) }}>
                  {initial(a.appName)}
                </span>
                <span className="app-text">
                  <span className="app-name">{a.appName}</span>
                  <span className="app-meter">
                    <span style={{ width: `${Math.max(3, (a.seconds / max) * 100)}%`, background: tagColor(tag) }} />
                  </span>
                </span>
              </button>
              <select
                className="tag-select"
                value={a.categoryId ?? ""}
                onChange={(e) => assign(a.appId, e.target.value)}
                aria-label={`${a.appName} kategorisi`}
              >
                <option value="">{UNCATEGORIZED}</option>
                {categories.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
              <span className="row-time">{formatDuration(a.seconds)}</span>
              <span className="chev">
                <IconChevron size={14} />
              </span>
            </div>
            {isOpen && (
              <ul className="titles">
                {titles.slice(0, MAX_TITLES).map((t) => (
                  <li key={t.key}>
                    <span className="title" title={t.label}>
                      {t.label || <em className="muted">(başlık okunamadı)</em>}
                    </span>
                    <span className="row-time">{formatDuration(t.seconds)}</span>
                  </li>
                ))}
                {titles.length > MAX_TITLES && (
                  <li className="muted">+{titles.length - MAX_TITLES} başlık daha</li>
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
    <ul className="legend">
      {order.map((id) => {
        const tag = id ? tags.get(id) : undefined;
        return (
          <li key={id ?? "none"}>
            <i style={{ background: tagColor(tag) }} />
            {tag?.name ?? UNCATEGORIZED}
          </li>
        );
      })}
    </ul>
  );
}

function initial(name: string): string {
  const ch = [...name.trim()][0] ?? "?";
  return ch.toLocaleUpperCase("tr-TR");
}
