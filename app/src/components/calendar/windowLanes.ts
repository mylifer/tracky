import type { CallSpan, WindowSpan } from "../../api";

/** Aynı pencereye bu kadar içinde dönülürse tek öğedir (arada geçen süre öğeye sayılmaz). */
export const JOIN_GAP_MS = 5 * 60_000;
/** Toplamı bundan kısa pencere ayrıntı sütununda gösterilmez (süresi blokta kalır). */
export const MIN_ITEM_MS = 60_000;
/** Öğeler arasında bırakılan dikey boşluk (piksel). */
const GAP_PX = 2;

/**
 * Ayrıntı sütununun öğesi: bir pencere (uygulama + başlık; aralıklı dönüşler birleşir) ya da
 * bir görüşme. `parts` gerçekten geçen aralıklardır; `start`–`end` arasındaki boşluklar başka
 * pencerelerde geçmiştir.
 */
export type WindowItem = {
  key: string;
  kind: "window" | "call";
  appId: string;
  appName: string;
  /** Pencere başlığı (kayıttaki gibi; düzenleme kapsamı bununla eşleşir); görüşmede boş. */
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

/** `[a, b)` aralığından `cuts` aralıklarının dışında kalan parçalar. */
function subtract(a: number, b: number, cuts: [number, number][]): [number, number][] {
  let parts: [number, number][] = [[a, b]];
  for (const [x, y] of cuts) {
    parts = parts.flatMap(([p, q]): [number, number][] =>
      y <= p || x >= q
        ? [[p, q]]
        : (
            [
              [p, x],
              [y, q],
            ] as [number, number][]
          ).filter(([u, v]) => v > u),
    );
  }
  return parts;
}

/**
 * Günün `[from, to)` aralığındaki pencereler ve görüşmeler, ayrıntı sütununun öğeleri olarak.
 * Aynı uygulama ve başlığa `JOIN_GAP_MS` içinde dönülen aralıklar tek öğedir; toplamı
 * `MIN_ITEM_MS`'ten kısa pencereler çıkar. Görüşme uygulamasının görüşme sürerken öne gelen
 * pencereleri (toplantı penceresi, menüleri) görüşmenin kendisidir, ayrıca gösterilmez;
 * tarayıcıdaki görüşmede sekmeler başka iş olabilir, kalır. Başlangıca göre sıralı.
 */
export function windowItems(windows: WindowSpan[], calls: CallSpan[], from: number, to: number): WindowItem[] {
  type Open = WindowItem & { cats: Map<string | null, number>; projects: Map<string | null, number> };
  const open = new Map<string, Open>();
  const done: Open[] = [];
  const inCall = new Map<string, [number, number][]>();
  for (const c of calls) {
    if (c.browser) continue;
    inCall.set(c.appId, [...(inCall.get(c.appId) ?? []), [Date.parse(c.start), Date.parse(c.end)]]);
  }
  const spans = windows
    .flatMap((w) =>
      subtract(Math.max(Date.parse(w.start), from), Math.min(Date.parse(w.end), to), inCall.get(w.appId) ?? []).map(
        ([a, b]) => ({ w, a, b }),
      ),
    )
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
        key: `w:${id}\u0000${a}`,
        kind: "window",
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
  const items: WindowItem[] = done
    .filter((i) => i.ms >= MIN_ITEM_MS)
    .map(({ cats, projects, ...i }) => ({ ...i, categoryId: dominant(cats), projectId: dominant(projects) }));
  for (const c of calls) {
    const a = Math.max(Date.parse(c.start), from);
    const b = Math.min(Date.parse(c.end), to);
    if (b <= a) continue;
    items.push({
      key: `c:${c.appId}\u0000${a}`,
      kind: "call",
      appId: c.appId,
      appName: c.appName,
      title: "",
      domain: null,
      start: a,
      end: b,
      ms: b - a,
      parts: [[a, b]],
      categoryId: null,
      projectId: null,
    });
  }
  return items.sort((x, y) => x.start - y.start || x.key.localeCompare(y.key));
}

/** Öğenin gösterilen adı. */
export function itemLabel(i: WindowItem): string {
  return i.kind === "call" ? `${i.appName} görüşmesi` : i.title.trim() || i.appName;
}

export type PlacedItem = WindowItem & {
  lane: number;
  top: number;
  /** Çubuğun yüksekliği: başlangıçtan bitişe (pencerede en az etiket kadar). */
  height: number;
};

/**
 * Öğeleri yan yana `lanes` şeride yerleştirir: her öğe başladığı saatte, çubuğu bittiği saate
 * kadar; etiketi (`labelPx`) ya da çubuğu başka öğeyle çakışan öğe sonraki şeride kayar.
 * Görüşmenin çubuğu şeritlerin solundaki raydadır, şeritte yalnızca etiketi yer kaplar (saatlerce
 * süren görüşme bir şeridi kapatmasın). Önce görüşmeler, sonra uzundan kısaya yerleşir; hiçbir
 * şeride sığmayan kısa öğeler gösterilmez (`hidden`) — yakınlaştıkça ya da sütun genişledikçe
 * görünür olurlar.
 */
export function placeItems(
  items: WindowItem[],
  top: (t: number) => number,
  lanes: number,
  labelPx: number,
): { placed: PlacedItem[]; hidden: number } {
  const order = [...items].sort(
    (x, y) => (x.kind === "call" ? 0 : 1) - (y.kind === "call" ? 0 : 1) || y.ms - x.ms || x.start - y.start,
  );
  const taken: [number, number][][] = Array.from({ length: Math.max(1, lanes) }, () => []);
  const placed: PlacedItem[] = [];
  let hidden = 0;
  for (const item of order) {
    const y0 = top(item.start);
    const bar = top(item.end) - y0;
    const y1 = y0 + (item.kind === "call" ? labelPx : Math.max(labelPx, bar));
    const lane = taken.findIndex((spans) => spans.every(([a, b]) => y1 + GAP_PX <= a || b + GAP_PX <= y0));
    if (lane < 0) {
      hidden++;
      continue;
    }
    taken[lane].push([y0, y1]);
    placed.push({ ...item, lane, top: y0, height: item.kind === "call" ? bar : Math.max(labelPx, bar) });
  }
  placed.sort((x, y) => x.start - y.start || x.lane - y.lane);
  return { placed, hidden };
}
