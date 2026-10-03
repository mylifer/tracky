import { useEffect, useState } from "react";
import { api, type UpdateStatus } from "../api";

/** Güncelleme durumu; arka plandaki denetimlerle canlı güncellenir. */
export function useUpdate(): [UpdateStatus | null, (s: UpdateStatus) => void] {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    api.updateStatus().then(setStatus, () => {});
    const off = api.onUpdate(setStatus);
    return () => {
      off.then((f) => f());
    };
  }, []);
  return [status, setStatus];
}
