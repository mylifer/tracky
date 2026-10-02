import type { Tag } from "../api";

/** Kategorisiz süre için nötr renk ve ad. */
export const UNCATEGORIZED = "Kategorisiz";
export const NO_PROJECT = "Projesiz";

/** Etiketin renk değişkeni; kategorisiz = nötr gri. Renk sıraya değil varlığa bağlıdır. */
export function tagColor(tag: Tag | undefined): string {
  return tag ? `var(--c${tag.color})` : "var(--c0)";
}

export function tagMap(tags: Tag[]): Map<string, Tag> {
  return new Map(tags.map((t) => [t.id, t]));
}

/** Yeni etikete henüz kullanılmayan ilk renk yuvası (hepsi doluysa en az kullanılan). */
export function nextColor(tags: Tag[]): number {
  const used = new Map<number, number>();
  for (const t of tags) used.set(t.color, (used.get(t.color) ?? 0) + 1);
  for (let c = 1; c <= 8; c++) if (!used.has(c)) return c;
  return [...used.entries()].sort((a, b) => a[1] - b[1])[0][0];
}
