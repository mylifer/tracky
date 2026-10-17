import { useRef } from "react";

/**
 * Üst üste giden isteklerden yalnızca en sonuncusunun yanıtı işlensin: dönem ya da filtre hızlı
 * değişince eski isteğin geç gelen yanıtı yenisinin üstüne yazılmasın. `begin()` yeni istek
 * açar; döndürdüğü işlev o istek hâlâ en yenisi mi söyler.
 */
export function latestGate(): () => () => boolean {
  let seq = 0;
  return () => {
    const n = ++seq;
    return () => n === seq;
  };
}

/** Bileşen boyunca aynı kalan [`latestGate`]. */
export function useLatest() {
  const gate = useRef<ReturnType<typeof latestGate> | null>(null);
  gate.current ??= latestGate();
  return gate.current;
}
