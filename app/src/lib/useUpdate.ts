import { useEffect, useState } from "react";
import { api, type UpdateStatus } from "../api";
import { useTauriEvent } from "./useTauriEvent";

/** Pencere öne gelince, son denetim bundan eskiyse yeniden denetlenir. */
const FOCUS_RECHECK_MS = 2 * 60_000;
/** Kancayı kullanan birden çok bileşen aynı odakta ikinci kez denetlemesin. */
let lastAsked = 0;

/**
 * Güncelleme durumu; arka plandaki denetimlerle canlı güncellenir. Pencere öne gelince son
 * denetim eskiyse yeniden denetler: yeni sürüm kenar çubuğunda hemen görünsün.
 */
export function useUpdate(): [UpdateStatus | null, (s: UpdateStatus) => void] {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    api.updateStatus().then(setStatus, () => {});
    const recheck = () => {
      if (document.visibilityState !== "visible" || Date.now() - lastAsked < FOCUS_RECHECK_MS) return;
      lastAsked = Date.now();
      api.updateStatus().then(
        (s) => {
          const last = s.lastChecked ? Date.parse(s.lastChecked) : 0;
          if (!s.checking && Date.now() - last >= FOCUS_RECHECK_MS) api.checkUpdate().catch(() => {});
        },
        () => {},
      );
    };
    window.addEventListener("focus", recheck);
    document.addEventListener("visibilitychange", recheck);
    return () => {
      window.removeEventListener("focus", recheck);
      document.removeEventListener("visibilitychange", recheck);
    };
  }, []);
  useTauriEvent(api.onUpdate, setStatus);
  return [status, setStatus];
}
