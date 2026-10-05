/** Toplamı bundan kısa pencere/sekme ayrıntıda adıyla görünmez (süresi toplamlarda kalır). */
export const MINOR_SECS = 5 * 60;

/** Kısa pencerelerin birlikte anlam taşıdığı ortak bağ: aynı proje ya da (projesizse) aynı site. */
export type MinorBond = { key: string; label: string; projectId?: string; domain?: string };

/** Ayrıntı satırı: tek pencere, aynı bağdaki kısa pencerelerin toplamı ya da "Diğer". */
export type DetailRow<T> =
  | { kind: "item"; item: T; seconds: number }
  | { kind: "group"; key: string; label: string; bond: MinorBond | null; items: T[]; seconds: number };

export const OTHER_KEY = "\u0000diğer";

/**
 * Pencereleri ayrıntı satırlarına çevirir: `MINOR_SECS`'ten uzun olanlar tek tek; kısa olanlar
 * ortak bağlarında (aynı proje/site) birlikte `MINOR_SECS`'i buluyorsa tek satırda, kalanı en
 * sonda "Diğer" satırında. `max` verilirse ilk `max - 1` satırdan sonrası da "Diğer"e katılır.
 * Satırlar uzundan kısaya; "Diğer" hep sonda. Girdi sırası (eşit sürede) korunur.
 */
export function detailRows<T>(
  items: T[],
  secs: (t: T) => number,
  bond: (t: T) => MinorBond | null = () => null,
  noun = "pencere",
  max?: number,
): DetailRow<T>[] {
  const rows: DetailRow<T>[] = [];
  const bonds = new Map<string, { bond: MinorBond; items: T[]; seconds: number }>();
  let other: T[] = [];
  for (const t of items) {
    const s = secs(t);
    if (s >= MINOR_SECS) {
      rows.push({ kind: "item", item: t, seconds: s });
      continue;
    }
    const b = bond(t);
    if (!b) {
      other.push(t);
      continue;
    }
    const g = bonds.get(b.key) ?? { bond: b, items: [], seconds: 0 };
    g.items.push(t);
    g.seconds += s;
    bonds.set(b.key, g);
  }
  for (const g of bonds.values()) {
    // Tek pencere ya da toplamı hâlâ kısa olan küme anlamlı bir bütün sayılmaz.
    if (g.items.length > 1 && g.seconds >= MINOR_SECS) {
      rows.push({
        kind: "group",
        key: `\u0000bağ:${g.bond.key}`,
        label: `${g.bond.label} · ${g.items.length} ${noun}`,
        bond: g.bond,
        items: g.items,
        seconds: g.seconds,
      });
    } else {
      other.push(...g.items);
    }
  }
  rows.sort((a, b) => b.seconds - a.seconds);
  if (max !== undefined && rows.length + (other.length ? 1 : 0) > max) {
    const extra = rows.splice(Math.max(0, max - 1));
    other = [...other, ...extra.flatMap((r) => (r.kind === "item" ? [r.item] : r.items))];
  }
  if (other.length) {
    rows.push({
      kind: "group",
      key: OTHER_KEY,
      label: `Diğer · ${other.length} ${noun}`,
      bond: null,
      items: other,
      seconds: other.reduce((n, t) => n + secs(t), 0),
    });
  }
  return rows;
}

/** Yalnızca "Diğer"den ibaret liste: üstündeki toplamı tekrarlar, göstermeye değmez. */
export const onlyOther = <T>(rows: DetailRow<T>[]) =>
  rows.length === 1 && rows[0].kind === "group" && rows[0].key === OTHER_KEY;
