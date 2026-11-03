import type { PlanBlock, WindowSpan } from "../api";

/**
 * Gün planı: saklanan zaman blokları (onaylı) ve henüz onaylanmamış zaman için önerilen taslak
 * bloklar. Onaylı blok yalnızca kullanıcı değiştirince değişir; taslak her yenilemede pencere
 * kaydından yeniden çıkarılır ama onaylı blokların aralığına girmez.
 *
 * Taslak (Rize'nin 15 dakikalık pencereleri ve ManicTime'ın "absorb"u gibi):
 * 1. Gün duvar saatine hizalı dilimlere bölünür; her dilimin etkinliği projelere göre toplanır.
 *    Etkinliği dilimin beşte birinden az olan dilim boştur.
 * 2. `breakMin`'den uzun boş süre günü parçalara ayırır (öğle arası gibi); daha kısa boşluklar
 *    bloğun içinde kalır.
 * 3. Parça içinde her dilim çoğunluk projesini alır; projesiz ve boş dilimler komşusununkini.
 * 4. `minBlockMin`'den kısa kısımlar uzun komşusuna katılır: kısa araya girmeler blok açmaz.
 * 5. Parçanın başı ilk etkinliğe, sonu son etkinliğe 5 dakikalık dilimle oturur.
 */

export const MIN = 60_000;
const SNAP = 5 * MIN;

export type PlanOpts = { slotMin: number; breakMin: number; minBlockMin: number };
export const PLAN_DEFAULTS: PlanOpts = { slotMin: 15, breakMin: 30, minBlockMin: 30 };

/** Çizimde ve düzenlemede kullanılan blok (zamanlar ms). `draft`: kaydedilmemiş öneri. */
export type Slot = { id: string; start: number; end: number; projectId: string | null; note: string; draft?: boolean };

/** Yeni onaylı bloğun kimliği. */
export const newId = () => crypto.randomUUID();

/** Taslağı onaylı bloğa çevirir. */
export const confirm = (b: Slot): Slot => ({ ...b, id: newId(), draft: undefined });

export const fromApi = (b: PlanBlock): Slot => ({
  id: b.id,
  start: Date.parse(b.start),
  end: Date.parse(b.end),
  projectId: b.projectId ?? null,
  note: b.note ?? "",
});

export const toApi = (b: Slot): PlanBlock => ({
  id: b.id,
  start: new Date(b.start).toISOString(),
  end: new Date(b.end).toISOString(),
  projectId: b.projectId,
  note: b.note,
});

const floorSnap = (t: number) => Math.floor(t / SNAP) * SNAP;
const ceilSnap = (t: number) => Math.ceil(t / SNAP) * SNAP;
export const snap = (t: number) => Math.round(t / SNAP) * SNAP;

/** Parçanın bir kısmı; `first`/`last`: içindeki ilk ve son etkinlik anı. */
type Run = {
  start: number;
  end: number;
  label: string | null;
  first: number;
  last: number;
  weight: Map<string, number>;
  active: number;
};

/** Kısmın kendi projesinde geçen süre (projesizse bütün etkinliği). */
const own = (r: Run) => (r.label ? (r.weight.get(r.label) ?? 0) : r.active);

function addWeight(into: Map<string, number>, from: Map<string, number>) {
  for (const [k, v] of from) into.set(k, (into.get(k) ?? 0) + v);
}

function lead(weight: Map<string, number>): string | null {
  let best: string | null = null;
  let most = 0;
  for (const [k, v] of weight) if (v > most || (v === most && best !== null && k < best)) [best, most] = [k, v];
  return best;
}

/** Aralıklardan `cut` aralıklarını çıkarır. */
function subtract(a: number, b: number, cut: { start: number; end: number }[]): [number, number][] {
  let parts: [number, number][] = [[a, b]];
  for (const c of cut) {
    parts = parts.flatMap(([x, y]): [number, number][] =>
      c.end <= x || c.start >= y
        ? [[x, y]]
        : (
            [
              [x, c.start],
              [c.end, y],
            ] as [number, number][]
          ).filter(([p, q]) => q > p),
    );
  }
  return parts;
}

/**
 * Onaylı blokların dışında kalan zaman için taslak bloklar (`draft`). `windows`: günün pencere
 * aralıkları (projesiyle); `now`: bundan sonrası çizilmez.
 */
export function draftSlots(
  windows: WindowSpan[],
  dayStart: number,
  dayEnd: number,
  now: number,
  confirmed: Slot[],
  opts: PlanOpts = PLAN_DEFAULTS,
): Slot[] {
  const end = Math.min(dayEnd, now);
  if (end <= dayStart) return [];
  const step = opts.slotMin * MIN;
  const n = Math.ceil((end - dayStart) / step);
  const slots = Array.from({ length: n }, () => ({
    weight: new Map<string, number>(),
    active: 0,
    first: Infinity,
    last: -Infinity,
  }));
  for (const w of windows) {
    const a = Math.max(Date.parse(w.start), dayStart);
    const b = Math.min(Date.parse(w.end), end);
    if (b <= a) continue;
    for (let i = Math.floor((a - dayStart) / step); i < n && dayStart + i * step < b; i++) {
      const s0 = dayStart + i * step;
      const x = Math.max(a, s0);
      const y = Math.min(b, s0 + step);
      if (y <= x) continue;
      const slot = slots[i];
      slot.active += y - x;
      slot.first = Math.min(slot.first, x);
      slot.last = Math.max(slot.last, y);
      if (w.projectId) slot.weight.set(w.projectId, (slot.weight.get(w.projectId) ?? 0) + (y - x));
    }
  }
  const busy = (i: number) => slots[i].active >= step / 5;

  // Uzun boşlukla ayrılan parçalar: [ilk dolu dilim, son dolu dilim].
  const pieces: [number, number][] = [];
  const gapSlots = Math.ceil(opts.breakMin / opts.slotMin);
  let open: [number, number] | null = null;
  for (let i = 0; i < n; i++) {
    if (!busy(i)) continue;
    if (open && i - open[1] - 1 < gapSlots) open[1] = i;
    else {
      if (open) pieces.push(open);
      open = [i, i];
    }
  }
  if (open) pieces.push(open);

  const out: Slot[] = [];
  for (const [i0, i1] of pieces) {
    // Dilim başına kısımlar; projesiz ya da boş dilim kendi kısmını açmaz.
    const runs: Run[] = [];
    for (let i = i0; i <= i1; i++) {
      const s = slots[i];
      const nonNull = [...s.weight.values()].reduce((x, y) => x + y, 0);
      const label = busy(i) && nonNull * 2 >= s.active ? lead(s.weight) : null;
      const r = runs[runs.length - 1];
      const start = dayStart + i * step;
      if (r && (label === null || r.label === null || r.label === label)) {
        r.end = start + step;
        if (r.label === null) r.label = label;
        r.first = Math.min(r.first, s.first);
        r.last = Math.max(r.last, s.last);
        addWeight(r.weight, s.weight);
        r.active += s.active;
      } else {
        runs.push({
          start,
          end: start + step,
          label,
          first: s.first,
          last: s.last,
          weight: new Map(s.weight),
          active: s.active,
        });
      }
    }
    const join = (into: Run, r: Run) => {
      into.start = Math.min(into.start, r.start);
      into.end = Math.max(into.end, r.end);
      into.first = Math.min(into.first, r.first);
      into.last = Math.max(into.last, r.last);
      addWeight(into.weight, r.weight);
      into.active += r.active;
    };
    // Kısa kısımlar komşusuna katılır; önce kendi projesinde en az süre geçen. İki yanı aynı
    // projeyse üçü tek blok olur (Kum'a 8 dakika SD App karışınca Kum bölünmesin), değilse
    // uzun komşuya.
    const short = opts.minBlockMin * MIN;
    const len = (r: Run) => r.end - r.start;
    while (runs.length > 1) {
      let k = -1;
      for (let m = 0; m < runs.length; m++) if (len(runs[m]) < short && (k < 0 || own(runs[m]) < own(runs[k]))) k = m;
      if (k < 0) break;
      const [a, r, b] = [runs[k - 1], runs[k], runs[k + 1]];
      if (a && b && a.label === b.label) {
        join(a, r);
        join(a, b);
        runs.splice(k, 2);
      } else {
        join(!a ? b : !b ? a : len(a) >= len(b) ? a : b, r);
        runs.splice(k, 1);
      }
      for (let m = 1; m < runs.length; m++) {
        if (runs[m].label === runs[m - 1].label) {
          join(runs[m - 1], runs[m]);
          runs.splice(m, 1);
          m--;
        }
      }
    }
    // Kenarlar gerçek etkinliğe oturur; farklı projeler arasındaki boşluk bir dilimden uzunsa
    // iki bloğa da katılmaz (boş kalır).
    runs[0].start = floorSnap(runs[0].first);
    runs[runs.length - 1].end = Math.min(ceilSnap(runs[runs.length - 1].last), end);
    for (let m = 1; m < runs.length; m++) {
      const [a, b] = [runs[m - 1], runs[m]];
      if (b.first - a.last >= step) {
        a.end = Math.min(ceilSnap(a.last), a.end);
        b.start = Math.max(floorSnap(b.first), b.start);
      }
    }
    for (const r of runs) {
      for (const [x, y] of subtract(r.start, r.end, confirmed)) {
        if (y - x >= 10 * MIN)
          out.push({ id: `draft:${x}`, start: x, end: y, projectId: r.label, note: "", draft: true });
      }
    }
  }
  return out;
}

/* ---------- Düzenleme: hepsi yeni liste döndürür, girdiyi değiştirmez. ---------- */

const sorted = (list: Slot[]) => [...list].sort((a, b) => a.start - b.start);

/** Aynı projeli, bitişik onaylı bloklar birleşir; 1 dakikadan kısalar düşer. */
export function normalize(list: Slot[]): Slot[] {
  const out: Slot[] = [];
  for (const b of sorted(list).filter((x) => x.end - x.start >= MIN)) {
    const last = out[out.length - 1];
    if (
      last &&
      last.projectId === b.projectId &&
      b.start - last.end <= MIN &&
      !(last.note && b.note && last.note !== b.note)
    ) {
      out[out.length - 1] = { ...last, end: Math.max(last.end, b.end), note: last.note || b.note };
    } else out.push(b);
  }
  return out;
}

/** `[a, b)` aralığını tek blok yapar: kesişen bloklar kırpılır ya da ikiye bölünür. */
export function paint(list: Slot[], a: number, b: number, projectId: string | null, note = ""): Slot[] {
  const out: Slot[] = [];
  for (const x of list) {
    if (x.end <= a || x.start >= b) out.push(x);
    else {
      if (x.start < a) out.push({ ...x, end: a });
      if (x.end > b) out.push({ ...x, id: x.start < a ? newId() : x.id, start: b });
    }
  }
  out.push({ id: newId(), start: a, end: b, projectId, note });
  return normalize(out);
}

/**
 * Bloğun kenarını `t`'ye taşır. Bitişik komşunun kenarı da birlikte kayar (ortak sınır);
 * blok ya da komşu 5 dakikadan kısalmaz, başka bloğun üstüne binilmez. Asla yeni blok doğmaz.
 */
export function resize(list: Slot[], id: string, edge: "start" | "end", t: number, now: number): Slot[] {
  const s = sorted(list);
  const i = s.findIndex((x) => x.id === id);
  if (i < 0) return list;
  const b = s[i];
  if (edge === "start") {
    const prev = s[i - 1];
    const touching = prev && b.start - prev.end <= MIN;
    const lo = prev ? (touching ? prev.start + SNAP : prev.end) : -Infinity;
    const v = Math.max(lo, Math.min(t, b.end - SNAP));
    return s.map((x, k) => (k === i ? { ...x, start: v } : touching && k === i - 1 ? { ...x, end: v } : x));
  }
  const next = s[i + 1];
  const touching = next && next.start - b.end <= MIN;
  const hi = next ? (touching ? next.end - SNAP : next.start) : now;
  const v = Math.min(hi, now, Math.max(t, b.start + SNAP));
  return s.map((x, k) => (k === i ? { ...x, end: v } : touching && k === i + 1 ? { ...x, start: v } : x));
}

/** Bloğu `t`'de ikiye böler (iki parça da en az 5 dakika); alttaki parça yeni bloktur. */
export function split(list: Slot[], id: string, t: number): Slot[] {
  const b = list.find((x) => x.id === id);
  if (!b || t - b.start < SNAP || b.end - t < SNAP) return list;
  return sorted([...list.filter((x) => x !== b), { ...b, end: t }, { ...b, id: newId(), start: t, note: "" }]);
}

/** Bloğu komşusuyla birleştirir: aradaki boşluk da kapanır, uzun olanın projesi kalır. */
export function mergeWith(list: Slot[], id: string, dir: "prev" | "next"): Slot[] {
  const s = sorted(list);
  const i = s.findIndex((x) => x.id === id);
  const j = dir === "prev" ? i - 1 : i + 1;
  if (i < 0 || j < 0 || j >= s.length) return list;
  const [a, b] = [s[i], s[j]];
  const keep = a.end - a.start >= b.end - b.start ? a : b;
  const merged = {
    ...keep,
    start: Math.min(a.start, b.start),
    end: Math.max(a.end, b.end),
    note: keep.note || (keep === a ? b.note : a.note),
  };
  return sorted([...s.filter((x) => x !== a && x !== b), merged]);
}

export const update = (list: Slot[], id: string, patch: Partial<Pick<Slot, "projectId" | "note">>) =>
  normalize(list.map((x) => (x.id === id ? { ...x, ...patch } : x)));

export const remove = (list: Slot[], id: string) => list.filter((x) => x.id !== id);

/**
 * Kaydederken oturumlarına proje yazılacak aralıklar: projesi belli (projesiz işareti dahil),
 * yeni ya da aralığı veya projesi değişen blokların değişen kısmı.
 */
export function assignRanges(before: Slot[], after: Slot[]): { start: number; end: number; projectId: string }[] {
  const out: { start: number; end: number; projectId: string }[] = [];
  for (const b of after) {
    if (!b.projectId) continue;
    // Önceki halde aynı projeyle kaplanmış kısımlar yeniden yazılmaz.
    const same = before.filter((x) => x.projectId === b.projectId);
    for (const [x, y] of subtract(b.start, b.end, same)) out.push({ start: x, end: y, projectId: b.projectId });
  }
  return out;
}
