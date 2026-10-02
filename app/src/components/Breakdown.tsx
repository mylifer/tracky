import { useEffect, useState } from "react";
import { api, formatDuration, type AppBucket, type Bucket, type Tag, type UsageTotal } from "../api";
import { NO_PROJECT, UNCATEGORIZED, tagColor } from "../lib/tags";

const MAX_TITLES = 15;

function pct(part: number, total: number) {
  return total > 0 ? Math.round((part / total) * 100) : 0;
}

/** Kategori ya da proje toplamları. Renk her zaman adla birlikte (yalnız renk değil). */
export function BucketList({
  buckets,
  tags,
  total,
  kind,
}: {
  buckets: Bucket[];
  tags: Map<string, Tag>;
  total: number;
  kind: "category" | "project";
}) {
  const max = buckets[0]?.seconds ?? 1;
  return (
    <ul className="list">
      {buckets.map((b) => {
        const tag = b.id ? tags.get(b.id) : undefined;
        const name = tag?.name ?? (kind === "category" ? UNCATEGORIZED : NO_PROJECT);
        return (
          <li key={b.id ?? "none"} className={b.id ? "" : "dim"}>
            <span className="name">
              <i className="swatch" style={{ background: tagColor(tag) }} />
              {name}
            </span>
            <span className="bar">
              <span style={{ width: `${Math.max(2, (b.seconds / max) * 100)}%`, background: tagColor(tag) }} />
            </span>
            <span className="time">
              {formatDuration(b.seconds)}
              <small>%{pct(b.seconds, total)}</small>
            </span>
          </li>
        );
      })}
    </ul>
  );
}

/** Uygulamalar: kategori atama ve tıklayınca pencere başlıkları. */
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
    <ul className="list apps">
      {apps.map((a) => {
        const tag = a.categoryId ? tags.get(a.categoryId) : undefined;
        const isOpen = open === a.appId;
        return (
          <li key={a.appId} className={isOpen ? "open" : ""}>
            <div className="app-row">
              <button
                className="row-toggle"
                onClick={() => setOpen(isOpen ? null : a.appId)}
                aria-expanded={isOpen}
                title={a.appId}
              >
                <span className="chevron">›</span>
                {a.appName}
              </button>
              <span className="bar">
                <span style={{ width: `${Math.max(2, (a.seconds / max) * 100)}%`, background: tagColor(tag) }} />
              </span>
              <span className="time">{formatDuration(a.seconds)}</span>
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
            </div>
            {isOpen && (
              <ul className="titles">
                {titles.slice(0, MAX_TITLES).map((t) => (
                  <li key={t.key}>
                    <span className="title" title={t.label}>
                      {t.label || <em className="muted">(başlık okunamadı)</em>}
                    </span>
                    <span className="time">{formatDuration(t.seconds)}</span>
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

/** Grafik efsanesi: raporda görünen kategoriler, sabit sırada. */
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
