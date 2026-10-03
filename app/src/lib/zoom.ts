import { useEffect, useRef } from "react";

/** Yakınlaştırma katsayısı sınırları ve düğmelerin adımları. */
export const ZOOM_MIN = 1;
export const ZOOM_MAX = 8;
const STEPS = [1, 1.5, 2, 3, 4, 6, 8];

export const clampZoom = (z: number) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z));

/** Düğme ve klavye için bir sonraki / önceki adım. */
export function stepZoom(z: number, dir: 1 | -1): number {
  return dir > 0
    ? (STEPS.find((s) => s > z + 0.01) ?? ZOOM_MAX)
    : ([...STEPS].reverse().find((s) => s < z - 0.01) ?? ZOOM_MIN);
}

/** Safari/WKWebView'in kıstırma olayı (lib.dom'da yok). */
type GestureEvent = UIEvent & { scale: number; clientX: number; clientY: number };

/**
 * Öğe üzerinde yakınlaştırma hareketleri: ⌘/Ctrl + tekerlek, Chrome/Edge'de iki parmakla
 * kıstırma (ctrl'li tekerlek olarak gelir) ve macOS WKWebView'de `gesture*` olayları.
 * `onZoom` çarpanı ve imlecin konumunu alır. `onPan` verilirse yatay kaydırma da yakalanır.
 * Öğe sonradan oluşabileceği için ref değil öğenin kendisi verilir (`ref={setEl}`).
 */
export function useZoomGestures(
  el: HTMLElement | null,
  onZoom: (factor: number, x: number, y: number) => void,
  onPan?: (dx: number) => void,
) {
  const zoom = useRef(onZoom);
  const pan = useRef(onPan);
  zoom.current = onZoom;
  pan.current = onPan;

  useEffect(() => {
    if (!el) return;
    let last = 1;
    const wheel = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        zoom.current(Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY);
      } else if (pan.current && Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
        e.preventDefault();
        pan.current(e.deltaX);
      }
    };
    const start = (e: Event) => {
      e.preventDefault();
      last = 1;
    };
    const change = (e: Event) => {
      e.preventDefault();
      const g = e as GestureEvent;
      zoom.current(g.scale / last, g.clientX, g.clientY);
      last = g.scale;
    };
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("gesturestart", start);
    el.addEventListener("gesturechange", change);
    return () => {
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("gesturestart", start);
      el.removeEventListener("gesturechange", change);
    };
  }, [el]);
}
