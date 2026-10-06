import type { Segment } from "../../api";
import { fromWallMs, wallMs } from "../../lib/dates";
import { HOUR_MS, MIN } from "./grid";

export type CategoryBucket = {
  start: number;
  end: number;
  /** En çok süren kategori. */
  categoryId: string | null;
  /** Aralığın takip edilen oranı (0–1). */
  coverage: number;
  /** Kategorilerin takip edilen süredeki payı, büyükten küçüğe. */
  shares: { id: string | null; share: number }[];
};

/** Aralığın bundan azı takip edildiyse şeritte boş kalır. */
const BUCKET_MIN_SHARE = 1 / 3;

/**
 * Günü `stepMin` dakikalık aralıklara böler (duvar saatiyle); her aralık o sürede en çok
 * süren kategoriyi alır. Takip edilmeyen aralıklar atlanır.
 */
export function categoryBuckets(segments: Segment[], dayStart: number, stepMin: number): CategoryBucket[] {
  const step = stepMin * MIN;
  const count = Math.ceil((24 * HOUR_MS) / step);
  const sums: Map<string | null, number>[] = Array.from({ length: count }, () => new Map());
  for (const s of segments) {
    const a = wallMs(+new Date(s.start), dayStart);
    const b = wallMs(+new Date(s.end), dayStart);
    for (let i = Math.max(0, Math.floor(a / step)); i < count && i * step < b; i++) {
      const ms = Math.min(b, (i + 1) * step) - Math.max(a, i * step);
      if (ms > 0) sums[i].set(s.categoryId, (sums[i].get(s.categoryId) ?? 0) + ms);
    }
  }
  const out: CategoryBucket[] = [];
  sums.forEach((m, i) => {
    const total = [...m.values()].reduce((x, y) => x + y, 0);
    if (total < step * BUCKET_MIN_SHARE) return;
    const shares = [...m.entries()].sort((x, y) => y[1] - x[1]).map(([id, ms]) => ({ id, share: ms / total }));
    out.push({
      start: fromWallMs(i * step, dayStart),
      end: fromWallMs((i + 1) * step, dayStart),
      categoryId: shares[0].id,
      coverage: Math.min(1, total / step),
      shares,
    });
  });
  return out;
}
