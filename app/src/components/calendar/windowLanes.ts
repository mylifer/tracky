import type { WindowSpan } from "../../api";

/**
 * Aynı pencereye bu kadar içinde dönülürse tek öğedir (arada geçen süre öğeye sayılmaz): toplantı
 * penceresi arada başka işe bakılsa da toplantı boyunca tek çubuk (Timely gibi). Zaman çizelgesinin
 * birleştirme aralığıyla (`timesheet::MERGE_GAP`) aynı.
 */
export const JOIN_GAP_MS = 15 * 60_000;
/** Toplamı bundan kısa pencere ayrıntı sütununda gösterilmez (süresi blokta kalır). */
export const MIN_ITEM_MS = 60_000;
/** Öğeler arasında bırakılan dikey boşluk (piksel). */
const GAP_PX = 2;

/**
 * Ayrıntı sütununun öğesi: bir pencere (uygulama + başlık; aralıklı dönüşler birleşir).
 * `parts` gerçekten geçen aralıklardır; `start`–`end` arasındaki boşluklar başka pencerelerde
 * geçmiştir.
 */
export type WindowItem = {
  key: string;
  appId: string;
  appName: string;
  /** Pencere başlığı (kayıttaki gibi; düzenleme kapsamı bununla eşleşir). */
  title: string;
  domain: string | null;
  start: number;
  end: number;
  ms: number;
  parts: [number, number][];
  /** En çok süren kategori ve proje (pencere bazında elle atama farklı olabilir). */
  categoryId: string | null;
  projectId: string | null;
};

function dominant(ms: Map<string | null, number>): string | null {
  let best: string | null = null;
  let most = -1;
  for (const [id, n] of ms) {
    if (n > most) [best, most] = [id, n];
  }
  return best;
}

/**
 * Günün `[from, to)` aralığındaki pencereler, ayrıntı sütununun öğeleri olarak. Aynı uygulama
 * ve başlığa `JOIN_GAP_MS` içinde dönülen aralıklar tek öğedir; toplamı `MIN_ITEM_MS`'ten kısa
 * pencereler çıkar. Başlangıca göre sıralı.
 */
export function windowItems(windows: WindowSpan[], from: number, to: number): WindowItem[] {
  type Open = WindowItem & { cats: Map<string | null, number>; projects: Map<string | null, number> };
  const open = new Map<string, Open>();
  const done: Open[] = [];
  const spans = windows
    .map((w) => ({ w, a: Math.max(Date.parse(w.start), from), b: Math.min(Date.parse(w.end), to) }))
    .filter((s) => s.b > s.a)
    .sort((x, y) => x.a - y.a);
  for (const { w, a, b } of spans) {
    const id = `${w.appId}\u0000${w.title}`;
    let item = open.get(id);
    if (item && a - item.end > JOIN_GAP_MS) {
      done.push(item);
      item = undefined;
    }
    if (!item) {
      item = {
        key: `${id}\u0000${a}`,
        appId: w.appId,
        appName: w.appName,
        title: w.title,
        domain: w.domain,
        start: a,
        end: b,
        ms: 0,
        parts: [],
        categoryId: null,
        projectId: null,
        cats: new Map(),
        projects: new Map(),
      };
      open.set(id, item);
    }
    const last = item.parts[item.parts.length - 1];
    if (last && a <= last[1]) last[1] = Math.max(last[1], b);
    else item.parts.push([a, b]);
    item.end = Math.max(item.end, b);
    item.ms += b - a;
    item.cats.set(w.categoryId, (item.cats.get(w.categoryId) ?? 0) + b - a);
    item.projects.set(w.projectId, (item.projects.get(w.projectId) ?? 0) + b - a);
  }
  done.push(...open.values());
  return done
    .filter((i) => i.ms >= MIN_ITEM_MS)
    .map(({ cats, projects, ...i }) => ({ ...i, categoryId: dominant(cats), projectId: dominant(projects) }))
    .sort((x, y) => x.start - y.start || x.key.localeCompare(y.key));
}

/** Öğenin gösterilen adı. */
export function itemLabel(i: { appName: string; title: string }): string {
  return i.title.trim() || i.appName;
}

export type PlacedItem = WindowItem & {
  lane: number;
  top: number;
  /** Çubuğun yüksekliği: başlangıçtan bitişe (en az etiket kadar). */
  height: number;
};

/**
 * Öğeleri yan yana `lanes` şeride yerleştirir: her öğe başladığı saatte, çubuğu bittiği saate
 * kadar; etiketi (`labelPx`) ya da çubuğu başka öğeyle çakışan öğe sonraki şeride kayar.
 * Uzundan kısaya yerleşir; hiçbir şeride sığmayan kısa öğeler gösterilmez (`hidden`) —
 * yakınlaştıkça ya da sütun genişledikçe görünür olurlar.
 */
export function placeItems(
  items: WindowItem[],
  top: (t: number) => number,
  lanes: number,
  labelPx: number,
): { placed: PlacedItem[]; hidden: number } {
  const order = [...items].sort((x, y) => y.ms - x.ms || x.start - y.start);
  const taken: [number, number][][] = Array.from({ length: Math.max(1, lanes) }, () => []);
  const placed: PlacedItem[] = [];
  let hidden = 0;
  for (const item of order) {
    const y0 = top(item.start);
    const bar = top(item.end) - y0;
    const y1 = y0 + Math.max(labelPx, bar);
    const lane = taken.findIndex((spans) => spans.every(([a, b]) => y1 + GAP_PX <= a || b + GAP_PX <= y0));
    if (lane < 0) {
      hidden++;
      continue;
    }
    taken[lane].push([y0, y1]);
    placed.push({ ...item, lane, top: y0, height: y1 - y0 });
  }
  placed.sort((x, y) => x.start - y.start || x.lane - y.lane);
  return { placed, hidden };
}

/** Başlangıca göre sıralı pencere aralıkları (unix ms) ve en uzununun süresi. */
type Sorted = { order: number[]; a: number[]; b: number[]; longest: number };
const sortedCache = new WeakMap<WindowSpan[], Sorted>();

function sorted(windows: WindowSpan[]): Sorted {
  let s = sortedCache.get(windows);
  if (!s) {
    const order = windows.map((_, i) => i).sort((x, y) => Date.parse(windows[x].start) - Date.parse(windows[y].start));
    const a = order.map((i) => Date.parse(windows[i].start));
    const b = order.map((i) => Date.parse(windows[i].end));
    s = { order, a, b, longest: b.reduce((m, end, i) => Math.max(m, end - a[i]), 0) };
    sortedCache.set(windows, s);
  }
  return s;
}

/**
 * `[start, end)` aralığında en çok süren pencere (uygulama + başlık): takvim bloğunun adı
 * (Timely gibi; blok uygulama akışından çıkar). Aralıkta pencere yoksa `null`.
 */
export function topWindow(
  windows: WindowSpan[],
  start: number,
  end: number,
): { appId: string; appName: string; title: string } | null {
  const { order, a, b, longest } = sorted(windows);
  // Aralıktan önce başlayıp içine taşabilecek ilk pencere (ikili arama).
  let lo = 0;
  let hi = a.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (a[mid] < start - longest) lo = mid + 1;
    else hi = mid;
  }
  const ms = new Map<string, number>();
  let best: WindowSpan | null = null;
  let most = 0;
  for (let k = lo; k < a.length && a[k] < end; k++) {
    const d = Math.min(b[k], end) - Math.max(a[k], start);
    if (d <= 0) continue;
    const w = windows[order[k]];
    const key = `${w.appId}\u0000${w.title}`;
    const n = (ms.get(key) ?? 0) + d;
    ms.set(key, n);
    if (n > most) [best, most] = [w, n];
  }
  return best && { appId: best.appId, appName: best.appName, title: itemLabel(best) };
}
