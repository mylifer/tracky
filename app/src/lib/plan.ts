import type { PlanBlock, WindowSpan } from "../api";
import { NO_PROJECT as NO_PROJECT_ID } from "../api";

/**
 * Gün planı (davranış kuralları: docs/gun-plani.md). İki tür blok:
 * - Onaylı: kullanıcının bloğu; yalnızca kullanıcı değiştirince değişir, hiçbir işlem başka bir
 *   bloğu kendiliğinden değiştirmez (kural 4, 5, 7). Zaman çizelgesine gider (kural 6).
 * - Öneri (`draft`): takvim etkinliğinden (kural 1, her zaman) ya da "Öner" düğmesine basınca
 *   bilgisayar kaydından (kural 3). Onaylanmadıkça hiçbir yere yazılmaz.
 *
 * Bilgisayar kaydından öneri ([`draftSlots`], Rize'nin 15 dakikalık pencereleri gibi):
 * 1. Gün duvar saatine hizalı dilimlere bölünür; her dilimin etkinliği projelere göre toplanır.
 *    Etkinliği dilimin beşte birinden az olan dilim boştur.
 * 2. `breakMin`'den uzun boş süre günü parçalara ayırır; daha kısa boşluklar bloğun içinde kalır.
 * 3. Her dilim çoğunluk projesini ve işini (baskın pencere başlığı) alır.
 * 4. `minBlockMin`'den kısa kısımlar komşusuna katılır.
 * 5. Parçanın başı ilk etkinliğe, sonu son etkinliğe 5 dakikalık dilimle oturur.
 */

export const MIN = 60_000;

/**
 * Gün planının uygulandığı ilk gün (1 Kasım 2026): öncesindeki günler eski görünümde kalır,
 * süreleri değişmez (çekirdekte `store::plan_since`). Uçtan uca testler `kum.planSince` ile
 * değiştirir (sabit "şimdi"leri Ekim'de).
 */
export function planSince(): number {
  try {
    const v = localStorage.getItem("kum.planSince");
    if (v) return +new Date(`${v}T00:00:00`);
  } catch {
    /* depolama kapalı */
  }
  return +new Date(2026, 10, 1);
}

/** Gün planı bu güne uygulanır mı. */
export const planApplies = (day: Date) => +day >= planSince();
const SNAP = 5 * MIN;

/** `names`: bu adlardan biri kısmın baskın işiyse önerinin adı o olur (öğrenen modelin tahmini). */
export type PlanOpts = { slotMin: number; breakMin: number; minBlockMin: number; names?: Set<string> };
export const PLAN_DEFAULTS: PlanOpts = { slotMin: 15, breakMin: 30, minBlockMin: 30 };

/**
 * Çizimde ve düzenlemede kullanılan blok (zamanlar ms). `draft`: kaydedilmemiş öneri;
 * `meeting`: takvim etkinliğinden gelen önerinin anahtarı (silinince o gün bir daha gelmez).
 */
export type Slot = {
  id: string;
  start: number;
  end: number;
  projectId: string | null;
  note: string;
  draft?: boolean;
  meeting?: string;
};

/** Yeni onaylı bloğun kimliği. */
export const newId = () => crypto.randomUUID();

/** Taslağı onaylı bloğa çevirir. */
export const confirm = (b: Slot): Slot => ({ ...b, id: newId(), draft: undefined, meeting: undefined });

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
  /** Kısmın işi: projede baskın pencere başlığı; karışıksa `null` (komşusuyla uyuşur). */
  task: string | null;
  first: number;
  last: number;
  weight: Map<string, number>;
  active: number;
};

/** Aynı iş mi: aynı proje ve (biri karışık değilse) aynı baskın başlık. */
const same = (a: Pick<Run, "label" | "task">, b: Pick<Run, "label" | "task">) =>
  a.label === b.label && (a.task === null || b.task === null || a.task === b.task);

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
export function subtract(a: number, b: number, cut: { start: number; end: number }[]): [number, number][] {
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
 * Bilgisayar kaydından öneriler (`draft`; kural 3). `windows`: günün pencere aralıkları
 * (projesiyle); `exclude`: öneri girmeyecek aralıklar (onaylı bloklar ve takvim etkinlikleri,
 * kural 2); `now`: bundan sonrası çizilmez.
 */
export function draftSlots(
  windows: WindowSpan[],
  dayStart: number,
  dayEnd: number,
  now: number,
  exclude: { start: number; end: number }[],
  opts: PlanOpts = PLAN_DEFAULTS,
): Slot[] {
  const end = Math.min(dayEnd, now);
  if (end <= dayStart) return [];
  const step = opts.slotMin * MIN;
  const n = Math.ceil((end - dayStart) / step);
  const slots = Array.from({ length: n }, () => ({
    weight: new Map<string, number>(),
    /** Proje ve pencere başlığına göre süre (anahtar: "proje\u0000başlık"). */
    tasks: new Map<string, number>(),
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
      if (w.projectId) {
        slot.weight.set(w.projectId, (slot.weight.get(w.projectId) ?? 0) + (y - x));
        const key = `${w.projectId}\u0000${w.title.trim()}`;
        slot.tasks.set(key, (slot.tasks.get(key) ?? 0) + (y - x));
      }
    }
  }
  const busy = (i: number) => slots[i].active >= step / 5;
  /**
   * Dilimin işi: projesinde süresinin en az yarısı tek bir pencere başlığındaysa o başlık
   * (Figma'da "Onboarding UI/UX"), değilse karışık. Aynı projede iş değişince ayrı blok açılır;
   * sekmeden sekmeye geçilen karışık dilimler komşusuna katılır, bloğu parçalamaz.
   */
  const taskOf = (i: number, label: string | null): string | null => {
    if (!label) return null;
    const total = slots[i].weight.get(label) ?? 0;
    let best: [string, number] | null = null;
    for (const [k, v] of slots[i].tasks) {
      const [p, title] = k.split("\u0000");
      if (p === label && title && (!best || v > best[1])) best = [title, v];
    }
    return best && best[1] * 2 >= total ? best[0] : null;
  };

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
    // Dilim başına kısımlar. Dilimin çoğu bir projedeyse o projenin, değilse atanmamış
    // (`null`) kısmıdır: atanmamış süre yandaki projeye geçmez (Trumore'dan sonra tarayıcıda
    // geçen yarım saat Trumore bloğunu uzatmasın). Boş dilim (kısa boşluk) önceki kısımda kalır.
    const runs: Run[] = [];
    for (let i = i0; i <= i1; i++) {
      const s = slots[i];
      const nonNull = [...s.weight.values()].reduce((x, y) => x + y, 0);
      const label = busy(i) && nonNull * 2 >= s.active ? lead(s.weight) : null;
      const task = busy(i) ? taskOf(i, label) : null;
      const r = runs[runs.length - 1];
      const start = dayStart + i * step;
      if (r && (!busy(i) || same(r, { label, task }))) {
        r.task ??= task;
        r.end = start + step;
        r.first = Math.min(r.first, s.first);
        r.last = Math.max(r.last, s.last);
        addWeight(r.weight, s.weight);
        r.active += s.active;
      } else {
        runs.push({
          start,
          end: start + step,
          label,
          task,
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
      into.task ??= r.task;
    };
    // Kısa kısımlar komşusuna katılır; önce kendi projesinde en az süre geçen. Projeli kısım
    // `minBlockMin`'den kısaysa, atanmamış kısım ancak bir dilim kadarsa (bir mesaja, bir
    // sekmeye bakmak) katılır; atanmamış kısım önceki bloğa geçer (işin devamı). İki yanı aynı
    // projeyse üçü tek blok olur (Kum'a 8 dakika SD App karışınca Kum bölünmesin), değilse
    // uzun komşuya. Şu an süren son kısım katılmaz: blok dakika dakika büyüyüp küçülmesin.
    const len = (r: Run) => r.end - r.start;
    const live = (r: Run) => r === runs[runs.length - 1] && r.end >= end - step && end < dayEnd;
    const isShort = (r: Run) => !live(r) && (r.label === null ? len(r) <= step : len(r) < opts.minBlockMin * MIN);
    while (runs.length > 1) {
      let k = -1;
      for (let m = 0; m < runs.length; m++) if (isShort(runs[m]) && (k < 0 || own(runs[m]) < own(runs[k]))) k = m;
      if (k < 0) break;
      const [a, r, b] = [runs[k - 1], runs[k], runs[k + 1]];
      if (a && b && same(a, b)) {
        join(a, r);
        join(a, b);
        runs.splice(k, 2);
      } else {
        join(!a ? b : !b ? a : r.label === null ? a : len(a) >= len(b) ? a : b, r);
        runs.splice(k, 1);
      }
      for (let m = 1; m < runs.length; m++) {
        if (same(runs[m], runs[m - 1])) {
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
      for (const [x, y] of subtract(r.start, r.end, exclude)) {
        if (y - x >= 10 * MIN)
          out.push({
            id: `draft:${x}`,
            start: x,
            end: y,
            projectId: r.label,
            note: r.task && opts.names?.has(r.task) ? r.task : "",
            draft: true,
          });
      }
    }
  }
  return out;
}

/* ---------- Düzenleme: hepsi yeni liste döndürür, girdiyi değiştirmez. ---------- */

const sorted = (list: Slot[]) => [...list].sort((a, b) => a.start - b.start);

/**
 * Sıralar, 1 dakikadan kısa parçaları atar. Bitişik bloklar kendiliğinden birleşmez (aynı
 * projede olsalar da): blok yalnızca kullanıcı birleştirince birleşir ([`mergeWith`]).
 */
export function normalize(list: Slot[]): Slot[] {
  return sorted(list).filter((x) => x.end - x.start >= MIN);
}

/**
 * Sürükleyerek çizilen yeni blok (kural 7): yalnızca boş alana çizilir. `[a, b)` onaylı
 * blokların dışında kalan en uzun parçasına kırpılır; onaylı bloklar değişmez. Boş yer 5
 * dakikadan kısaysa liste aynen döner.
 */
export function draw(list: Slot[], a: number, b: number, projectId: string | null, note = ""): Slot[] {
  const free = subtract(a, b, list).sort((x, y) => y[1] - y[0] - (x[1] - x[0]))[0];
  if (!free || free[1] - free[0] < SNAP) return list;
  return sorted([...list, { id: newId(), start: free[0], end: free[1], projectId, note }]);
}

/**
 * Bloğun kenarını `t`'ye taşır (kural 4, 5): yalnızca bu blok değişir. Küçültünce açılan süre
 * boş kalır; büyütürken komşu bloğun kenarında durur. Blok 5 dakikadan kısalmaz, şimdiyi geçmez.
 */
export function resize(list: Slot[], id: string, edge: "start" | "end", t: number, now: number): Slot[] {
  const s = sorted(list);
  const i = s.findIndex((x) => x.id === id);
  if (i < 0) return list;
  const b = s[i];
  if (edge === "start") {
    const lo = s[i - 1]?.end ?? -Infinity;
    const v = Math.max(lo, Math.min(t, b.end - SNAP));
    return s.map((x, k) => (k === i ? { ...x, start: v } : x));
  }
  const hi = Math.min(s[i + 1]?.start ?? Infinity, now);
  const v = Math.min(hi, Math.max(t, b.start + SNAP));
  return s.map((x, k) => (k === i ? { ...x, end: v } : x));
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
 * Takvim etkinliklerinden öneriler (kural 1, 7. varsayılan): etkinlik tam saatinde ve süresince,
 * projesi toplantının projesi, adı konusu. Silinen (`dismissed`) ya da bir onaylı blokla
 * çakışan etkinlik öneri olmaz: onaylanınca ya da kullanıcı o saate blok koyunca kaybolur,
 * blok küçültülünce geri gelmez (kural 4).
 */
export function meetingSlots(
  meetings: {
    uid: string;
    start: string;
    end: string;
    subject: string;
    projectId?: string | null;
    ignored?: boolean;
    attendance?: { status: string } | null;
  }[],
  dayStart: number,
  dayEnd: number,
  confirmed: Slot[],
  dismissed: Set<string>,
): Slot[] {
  return (
    meetings
      .map((m) => {
        const start = Math.max(Date.parse(m.start), dayStart);
        const end = Math.min(Date.parse(m.end), dayEnd);
        return { m, start, end, key: `${m.uid}@${m.start}` };
      })
      // "Katılmadım" denen (ya da Kum'un katılmadın dediği) toplantı öneri olmaz.
      .filter(
        ({ m, start, end, key }) =>
          end - start >= SNAP && !m.ignored && m.attendance?.status !== "skipped" && !dismissed.has(key),
      )
      .filter(({ start, end }) => !confirmed.some((c) => c.start < end && start < c.end))
      .map(({ m, start, end, key }) => ({
        id: `meeting:${key}`,
        start,
        end,
        projectId: m.projectId ?? null,
        note: m.subject.trim(),
        draft: true,
        meeting: key,
      }))
  );
}

/**
 * Ekranda gösterilecek öneriler: saklanan bilgisayar önerileri onaylı blokların ve etkinlik
 * önerilerinin dışında kalan, en az 5 dakikalık parçalarıyla (kural 2, 4).
 */
export function visibleSuggestions(stored: Slot[], confirmed: Slot[], meetingsShown: Slot[]): Slot[] {
  const taken = [...confirmed, ...meetingsShown];
  return stored.flatMap((d) =>
    subtract(d.start, d.end, taken)
      .filter(([x, y]) => y - x >= SNAP)
      .map(([x, y]) => ({ ...d, id: `draft:${x}`, start: x, end: y })),
  );
}

/** Aynı projedeki iki öneri arasında bundan kısa boşluk varsa tek öneri olurlar (kural 3c). */
const ASSIGN_JOIN = 15 * MIN;

/**
 * Pencereler sütununda `[start, end)` projeye atandı (kural 3c): saklanan önerilerde o aralık
 * bu projeye geçer, aynı projede yakın öneriyle birleşir. Projesiz ya da `NO_PROJECT` atamada
 * aralıkta öneri kalmaz. Onaylı bloklar ve etkinlikler ekranda zaten çıkarılır
 * ([`visibleSuggestions`]); günün dışı kırpılır.
 */
export function withAssigned(
  stored: Slot[],
  start: number,
  end: number,
  projectId: string | null,
  dayStart: number,
  dayEnd: number,
): Slot[] {
  const a = Math.max(floorSnap(start), dayStart);
  const b = Math.min(ceilSnap(end), dayEnd);
  if (b - a < SNAP) return stored;
  const rest = stored.flatMap((d) =>
    subtract(d.start, d.end, [{ start: a, end: b }])
      .filter(([x, y]) => y - x >= SNAP)
      .map(([x, y]) => ({ ...d, start: x, end: y })),
  );
  if (!projectId || projectId === NO_PROJECT_ID) return sorted(rest);
  let slot: Slot = { id: newId(), start: a, end: b, projectId, note: "", draft: true };
  const keep: Slot[] = [];
  for (const d of sorted(rest)) {
    const near = d.projectId === projectId && d.start - slot.end < ASSIGN_JOIN && slot.start - d.end < ASSIGN_JOIN;
    if (near)
      slot = {
        ...slot,
        start: Math.min(slot.start, d.start),
        end: Math.max(slot.end, d.end),
        note: slot.note || d.note,
      };
    else keep.push(d);
  }
  return sorted([...keep, slot]);
}

/** Pencereler sütununda (ya da takvimde) bir aralık projeye atandı: Plan'a öneri olarak düşer. */
export const PLAN_ASSIGNED = "kum:plan-assigned";
export type PlanAssigned = { start: number; end: number; projectId: string | null };

/** Atamayı Plan sütununa bildirir (kural 3c). */
export function announceAssigned(detail: PlanAssigned) {
  window.dispatchEvent(new CustomEvent<PlanAssigned>(PLAN_ASSIGNED, { detail }));
}
