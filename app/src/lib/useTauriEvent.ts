import { useEffect, useRef } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * Bir Tauri olayına abone olur; işleyici her çizimde güncellenir, abonelik
 * yalnızca `subscribe` değişince yenilenir (eski kapanış yakalanmaz).
 */
export function useTauriEvent<T>(
  subscribe: (cb: (payload: T) => void) => Promise<UnlistenFn>,
  handler: (payload: T) => void,
) {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    const unlisten = subscribe((payload) => latest.current(payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, [subscribe]);
}
