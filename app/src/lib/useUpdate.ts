import { useEffect, useState } from "react";
import { api, type UpdateStatus } from "../api";
import { useTauriEvent } from "./useTauriEvent";

/** Güncelleme durumu; arka plandaki denetimlerle canlı güncellenir. */
export function useUpdate(): [UpdateStatus | null, (s: UpdateStatus) => void] {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    api.updateStatus().then(setStatus, () => {});
  }, []);
  useTauriEvent(api.onUpdate, setStatus);
  return [status, setStatus];
}
