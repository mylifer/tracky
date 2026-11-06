import type { WindowSpan } from "../api";

/**
 * Öneri modeli (docs/gun-plani.md kural 3a): kullanıcının geçmişte onayladığı bloklardan
 * öğrenir: hangi uygulama ve pencere başlığı hangi projeye ve hangi ada gitti. Bilgisayarda
 * çalışır, internete bağlanmaz.
 *
 * Tahmin sırası (en güvenilirden): aynı uygulama + aynı başlık; başlığın kelimeleri; yalnızca
 * uygulama (o uygulama hep aynı yere gidiyorsa). Hiçbiri yeterince güçlü değilse tahmin yoktur
 * ve pencerenin kendi (kuralların verdiği) projesi kullanılır (kural 3b).
 */

/** Geçmişten bir örnek: bir onaylı bloğun içindeki pencere (`get_plan_examples`). */
export type PlanExample = { appName: string; title: string; projectId: string; note: string; seconds: number };

/** Bir pencere için tahmin: proje ve ad. */
export type Guess = { projectId: string; note: string };

type Votes = Map<string, number>;

const label = (g: Guess) => `${g.projectId}\u0000${g.note}`;
const unlabel = (k: string): Guess => {
  const [projectId, note] = k.split("\u0000");
  return { projectId, note };
};

/** Başlığın anlamlı kelimeleri (küçük harf, en az 3 harf, sayılar hariç). */
export function words(title: string): string[] {
  return [
    ...new Set(
      title
        .toLocaleLowerCase("tr-TR")
        .split(/[^\p{L}\p{N}]+/u)
        .filter((w) => w.length >= 3 && !/^\d+$/.test(w)),
    ),
  ];
}

function vote(map: Map<string, Votes>, key: string, l: string, w: number) {
  let v = map.get(key);
  if (!v) map.set(key, (v = new Map()));
  v.set(l, (v.get(l) ?? 0) + w);
}

/** En çok oyu alan etiket ve payı (0–1). */
function best(v: Votes | undefined): [string, number] | null {
  if (!v) return null;
  let total = 0;
  let top: [string, number] | null = null;
  for (const [k, n] of v) {
    total += n;
    if (!top || n > top[1] || (n === top[1] && k < top[0])) top = [k, n];
  }
  return top && total > 0 ? [top[0], top[1] / total] : null;
}

export type Model = {
  exact: Map<string, Votes>;
  word: Map<string, Votes>;
  app: Map<string, Votes>;
  size: number;
};

export function buildModel(examples: PlanExample[]): Model {
  const m: Model = { exact: new Map(), word: new Map(), app: new Map(), size: examples.length };
  for (const e of examples) {
    const l = label(e);
    vote(m.exact, `${e.appName}\u0000${e.title.trim()}`, l, e.seconds);
    for (const w of words(e.title)) vote(m.word, w, l, e.seconds);
    vote(m.app, e.appName, l, e.seconds);
  }
  return m;
}

/** Pencerenin projesini ve adını tahmin eder; yeterince emin değilse `null`. */
export function guess(m: Model, appName: string, title: string): Guess | null {
  const exact = best(m.exact.get(`${appName}\u0000${title.trim()}`));
  if (exact && exact[1] >= 0.6) return unlabel(exact[0]);
  // Kelimeler: her kelimenin oyu, o kelimenin ne kadar ayırt edici olduğuyla (tek etikete
  // gidiyorsa tam, dağınıksa az) ağırlıklı toplanır.
  const total: Votes = new Map();
  let matched = 0;
  for (const w of words(title)) {
    const v = m.word.get(w);
    const b = best(v);
    if (!v || !b) continue;
    matched++;
    total.set(b[0], (total.get(b[0]) ?? 0) + b[1]);
  }
  const byWords = best(total);
  if (byWords && matched >= 1 && byWords[1] >= 0.6 && (total.get(byWords[0]) ?? 0) >= 0.8) return unlabel(byWords[0]);
  const app = best(m.app.get(appName));
  if (app && app[1] >= 0.9) return unlabel(app[0]);
  return null;
}

/**
 * Pencereleri modelin tahminiyle yeniden etiketler: tahmin varsa projesi tahminin projesi,
 * başlığı tahminin adı olur (aynı ada giden pencereler tek iş sayılır). Tahmin yoksa pencere
 * olduğu gibi kalır (kuralların verdiği proje).
 */
export function relabel(
  m: Model,
  windows: WindowSpan[],
): { windows: WindowSpan[]; guessed: number; names: Set<string> } {
  let guessed = 0;
  const names = new Set<string>();
  const out = windows.map((w) => {
    const g = guess(m, w.appName, w.title);
    if (!g) return w;
    guessed++;
    if (g.note) names.add(g.note);
    return { ...w, projectId: g.projectId, title: g.note || w.title };
  });
  return { windows: out, guessed, names };
}
