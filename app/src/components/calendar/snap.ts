import type { Meeting, WindowSpan } from "../../api";

/**
 * Blok kenarının sürüklenirken yapıştığı an: takvim toplantısının ya da büyük uygulama
 * kümesinin başı veya sonu.
 */
export type SnapTarget = {
  at: number;
  kind: "meeting" | "app";
  /** Toplantının konusu ya da uygulamanın adı. */
  name: string;
  /** Hedefin toplantının/kümenin başı mı sonu mu olduğu. */
  edge: "start" | "end";
};

const MIN = 60_000;
/** Aynı uygulamaya bu kadar içinde dönülürse küme sürer: arada kısa bakılan iş kümeyi bölmez. */
export const APP_GAP_MS = 5 * MIN;
/** Uygulamada en az bu kadar geçen küme büyük sayılır; sınırları yapışma noktasıdır. */
export const BIG_APP_MS = 15 * MIN;
/** Kenar hedefe bu kadar piksel yaklaşınca yapışır (5 dakikalık dilimden önce gelir). */
export const PULL_PX = 10;
/** Yapışan kenar ancak bu kadar piksel uzaklaşınca kurtulur: yapışma önceliklidir. */
export const RELEASE_PX = 18;
/** Uzaklaştırılmış takvimde çekim bu süreyi aşmaz: hedefler arası yerlere de varılabilsin. */
export const MAX_PULL_MS = 15 * MIN;

/**
 * `[from, to)` aralığındaki yapışma noktaları: toplantıların başı ve sonu, uygulamada
 * `BIG_APP_MS`'ten çok geçen kümelerin (aynı uygulamaya `APP_GAP_MS` içinde dönülen aralıklar)
 * başı ve sonu. Küme sınırları dakikaya dışa doğru yuvarlanır (küme bloğun içinde kalsın).
 * Aynı dakikaya düşen hedeflerden toplantınınki kalır. Zamana göre sıralı.
 */
export function snapTargets(meetings: Meeting[], windows: WindowSpan[], from: number, to: number): SnapTarget[] {
  const out: SnapTarget[] = [];
  const add = (at: number, kind: SnapTarget["kind"], name: string, edge: SnapTarget["edge"]) => {
    if (at >= from && at <= to) out.push({ at, kind, name, edge });
  };
  for (const m of meetings) {
    const name = m.subject.trim() || "(konusuz)";
    add(Date.parse(m.start), "meeting", name, "start");
    add(Date.parse(m.end), "meeting", name, "end");
  }

  type Cluster = { appName: string; start: number; end: number; ms: number };
  const open = new Map<string, Cluster>();
  const done: Cluster[] = [];
  const spans = windows
    .map((w) => ({ w, a: Math.max(Date.parse(w.start), from), b: Math.min(Date.parse(w.end), to) }))
    .filter((s) => s.b > s.a)
    .sort((x, y) => x.a - y.a);
  for (const { w, a, b } of spans) {
    let c = open.get(w.appId);
    if (c && a - c.end > APP_GAP_MS) {
      done.push(c);
      c = undefined;
    }
    if (!c) {
      c = { appName: w.appName, start: a, end: a, ms: 0 };
      open.set(w.appId, c);
    }
    // Üst üste binen aralıklar (iki bilgisayar) kümenin süresini iki kez saymasın.
    c.ms += Math.max(0, b - Math.max(a, c.end));
    c.end = Math.max(c.end, b);
  }
  done.push(...open.values());
  for (const c of done) {
    if (c.ms < BIG_APP_MS) continue;
    add(Math.floor(c.start / MIN) * MIN, "app", c.appName, "start");
    add(Math.ceil(c.end / MIN) * MIN, "app", c.appName, "end");
  }

  out.sort((x, y) => x.at - y.at || (x.kind === y.kind ? 0 : x.kind === "meeting" ? -1 : 1));
  return out.filter((t, i) => i === 0 || Math.round(t.at / MIN) !== Math.round(out[i - 1].at / MIN));
}

/**
 * Sürüklenen kenarın yapışacağı hedef: `raw`a `pullMs` kadar yakın, `ok` olanların en yakını.
 * Kenar zaten bir hedefe (`held`) yapışıksa `releaseMs` kadar uzaklaşmadıkça orada kalır.
 * Yakında hedef yoksa `null` (kenar dilime yuvarlanır).
 */
export function snapEdge(
  raw: number,
  targets: SnapTarget[],
  pullMs: number,
  releaseMs: number,
  held: SnapTarget | null,
  ok: (at: number) => boolean = () => true,
): SnapTarget | null {
  if (held && Math.abs(raw - held.at) <= releaseMs && ok(held.at)) return held;
  let best: SnapTarget | null = null;
  for (const t of targets) {
    const d = Math.abs(raw - t.at);
    if (d <= pullMs && ok(t.at) && (!best || d < Math.abs(raw - best.at))) best = t;
  }
  return best;
}

/** Ekrandaki yapışma mesafeleri (piksel) bu yakınlaştırmada kaç ms: çekim `MAX_PULL_MS`'i aşmaz. */
export function snapReach(hourPx: number): { pull: number; release: number } {
  const pull = Math.min((PULL_PX / hourPx) * 3600_000, MAX_PULL_MS);
  return { pull, release: (pull * RELEASE_PX) / PULL_PX };
}
