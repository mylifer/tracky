import type { WorkBlock } from "../api";

/**
 * Taslak: takvimin Oturumlar sütununda blokların göze göre birleştirilmiş hali. Bloklar (ve
 * zaman çizelgesine giden satırlar) değişmez; yalnızca çizim sadeleşir.
 *
 * - Aradaki boşluk eşikten kısa, projeleri uyuşan (aynı ya da biri projesiz) ardışık bloklar
 *   tek taslak olur.
 * - Etkin süresi eşikten kısa blok (oyun sırasında bir mesaja bakmak, toplantı arasında bir
 *   PDF açmak) yakınındaki komşusuna katılır: önce aynı projeli komşuya, yoksa uzun olana.
 *   Taslakta "araya giren" olarak görünür.
 */

/** Eşik seçenekleri (dakika); 0: taslak kapalı, her blok ayrı. */
export const DRAFT_STEPS = [0, 5, 10, 15, 30] as const;
export const DRAFT_DEFAULT = 15;

export type DraftGroup = {
  start: string;
  end: string;
  /** Taslağın blokları, sıralı. */
  blocks: WorkBlock[];
  activeSeconds: number;
  /** Taslağın en az yarısını kaplayan proje. */
  projectId: string | null;
  /** En çok süren kategori. */
  categoryId: string | null;
  topApps: WorkBlock["topApps"];
  switches: number;
  /** Taslağın projesinden başka projede (ya da projesiz) geçen bloklar: araya girenler. */
  others: WorkBlock[];
  /** Zaman çizelgesine gönderilmemiş blok sayısı (çizelgeye bağlı projelerde). */
  unsent: number;
  /** Çizelgeye bağlı bütün blokları gönderildi mi; bağlı blok yoksa `undefined`. */
  sent?: boolean;
};

type Run = { blocks: WorkBlock[]; start: number; end: number };

const ms = (s: string) => Date.parse(s);
const active = (r: Run) => r.blocks.reduce((n, b) => n + b.activeSeconds, 0);

/** En uzun süren anahtar; eşitlikte sözlük sırasıyla ilki (sonuç sıraya bağlı olmasın). */
function dominant(weights: Map<string, number>): [string, number] | null {
  let best: [string, number] | null = null;
  for (const [k, v] of weights) {
    if (!best || v > best[1] || (v === best[1] && k < best[0])) best = [k, v];
  }
  return best;
}

/** Bloklarda en çok süren proje (projesizler sayılmaz). */
function lead(blocks: WorkBlock[]): string | null {
  const w = new Map<string, number>();
  for (const b of blocks) if (b.projectId) w.set(b.projectId, (w.get(b.projectId) ?? 0) + b.activeSeconds);
  return dominant(w)?.[0] ?? null;
}

const fits = (a: string | null, b: string | null) => a === null || b === null || a === b;

function join(into: Run, other: Run) {
  into.blocks = [...into.blocks, ...other.blocks].sort((x, y) => ms(x.start) - ms(y.start));
  into.start = Math.min(into.start, other.start);
  into.end = Math.max(into.end, other.end);
}

/**
 * Blokları taslaklara ayırır (`minutes`: eşik). Eşik 0 ise her blok kendi taslağıdır.
 * Bloklar başlangıca göre sıralı olmayabilir.
 */
export function draftGroups(blocks: WorkBlock[], minutes: number): DraftGroup[] {
  const sorted = [...blocks].sort((a, b) => ms(a.start) - ms(b.start));
  if (minutes <= 0) return sorted.map((b) => group([b]));
  const gapMax = minutes * 60_000;
  const shortSec = minutes * 60;

  // Ardışık, projeleri uyuşan bloklar.
  const runs: Run[] = [];
  for (const b of sorted) {
    const r = runs[runs.length - 1];
    if (r && ms(b.start) - r.end < gapMax && fits(lead(r.blocks), b.projectId)) {
      join(r, { blocks: [b], start: ms(b.start), end: ms(b.end) });
    } else {
      runs.push({ blocks: [b], start: ms(b.start), end: ms(b.end) });
    }
  }

  // Kısa kısımlar komşusuna katılır; sonra uyuşan komşular birleşir. Değişiklik kalmayana dek.
  for (let changed = true; changed;) {
    changed = false;
    for (let i = 0; i < runs.length && !changed; i++) {
      const r = runs[i];
      if (active(r) >= shortSec) continue;
      const a = runs[i - 1];
      const b = runs[i + 1];
      const nearA = a && r.start - a.end < gapMax;
      const nearB = b && b.start - r.end < gapMax;
      if (!nearA && !nearB) continue;
      const p = lead(r.blocks);
      let into: Run;
      if (nearA && nearB) {
        if (p && lead(a.blocks) === p) into = a;
        else if (p && lead(b.blocks) === p) into = b;
        else into = active(a) >= active(b) ? a : b;
      } else into = nearA ? a : b;
      join(into, r);
      runs.splice(i, 1);
      changed = true;
    }
    for (let i = 1; i < runs.length && !changed; i++) {
      const a = runs[i - 1];
      const b = runs[i];
      if (b.start - a.end < gapMax && fits(lead(a.blocks), lead(b.blocks))) {
        join(a, b);
        runs.splice(i, 1);
        changed = true;
      }
    }
  }
  return runs.map((r) => group(r.blocks));
}

function group(blocks: WorkBlock[]): DraftGroup {
  const total = blocks.reduce((n, b) => n + b.activeSeconds, 0);
  const projects = new Map<string, number>();
  const categories = new Map<string, number>();
  const apps = new Map<string, { appId: string; appName: string; seconds: number }>();
  for (const b of blocks) {
    if (b.projectId) projects.set(b.projectId, (projects.get(b.projectId) ?? 0) + b.activeSeconds);
    if (b.categoryId) categories.set(b.categoryId, (categories.get(b.categoryId) ?? 0) + b.activeSeconds);
    for (const a of b.topApps) {
      const cur = apps.get(a.appName);
      if (cur) cur.seconds += a.seconds;
      else apps.set(a.appName, { ...a });
    }
  }
  const top = dominant(projects);
  const projectId = top && top[1] * 2 >= total && top[1] > 0 ? top[0] : null;
  const linked = blocks.filter((b) => b.sent !== undefined);
  const unsent = linked.filter((b) => b.sent === false).length;
  return {
    start: blocks[0].start,
    end: blocks.reduce((e, b) => (ms(b.end) > ms(e) ? b.end : e), blocks[0].end),
    blocks,
    activeSeconds: total,
    projectId,
    categoryId: dominant(categories)?.[0] ?? null,
    topApps: [...apps.values()].sort((a, b) => b.seconds - a.seconds || a.appName.localeCompare(b.appName)).slice(0, 3),
    switches: blocks.reduce((n, b) => n + b.switches, 0) + blocks.length - 1,
    others: blocks.filter((b) => b.projectId !== projectId),
    unsent,
    sent: linked.length ? unsent === 0 : undefined,
  };
}

/** Taslağı tek blok gibi gösterilecek şekle getirir (başlık, renk ve ipucu için). */
export function asBlock(g: DraftGroup): WorkBlock {
  return {
    start: g.start,
    end: g.end,
    activeSeconds: g.activeSeconds,
    categoryId: g.categoryId,
    projectId: g.projectId,
    switches: g.switches,
    topApps: g.topApps,
    sent: g.sent,
  };
}
