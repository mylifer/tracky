import { useEffect, useState } from "react";
import { api, type Taxonomy } from "../api";
import { CHANGED, useChanged } from "./feedback";

/** Önbellekteki sınıflandırma bu kadar eskiyse yeniden istenir (başka sayfada değişmiş olabilir). */
const MAX_AGE_MS = 30_000;

let cached: { at: number; taxonomy: Promise<Taxonomy> } | null = null;

// Kayıtlar değişince önbellek boşalır; bileşenlerin dinleyicisi bundan sonra çalışır ve
// hepsi tek bir yeni isteği paylaşır.
if (typeof window !== "undefined") window.addEventListener(CHANGED, () => (cached = null));

/** Tüm seçiciler aynı isteği paylaşır; hata olursa bir sonraki çağrı yeniden dener. */
export function loadTaxonomy(): Promise<Taxonomy> {
  if (!cached || Date.now() - cached.at > MAX_AGE_MS) {
    const taxonomy = api.taxonomy();
    const entry = { at: Date.now(), taxonomy };
    cached = entry;
    taxonomy.catch(() => {
      if (cached === entry) cached = null;
    });
  }
  return cached.taxonomy;
}

/**
 * Müşteriler ve proje → müşteri eşlemesi gibi sınıflandırma bilgisi; `enabled` kapalıysa istenmez.
 * Kayıtlar değişince (`notifyChanged`) yenilenir.
 */
export function useTaxonomy(enabled = true): Taxonomy | null {
  const [taxonomy, setTaxonomy] = useState<Taxonomy | null>(null);
  const [rev, setRev] = useState(0);
  useChanged(() => setRev((r) => r + 1));
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    loadTaxonomy().then(
      (t) => live && setTaxonomy(t),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [enabled, rev]);
  return taxonomy;
}
