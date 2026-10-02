import { useCallback, useEffect, useState } from "react";
import { api, type AppStatus } from "./api";
import Onboarding from "./Onboarding";
import Today from "./Today";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.status().then(setStatus, (e) => setError(String(e)));
  }, []);

  useEffect(refresh, [refresh]);

  if (error) return <main className="center"><p className="error">{error}</p></main>;
  if (!status) return null;

  const needsSetup = !status.onboarded || !status.accessibility;
  return needsSetup ? (
    <Onboarding status={status} onChange={refresh} />
  ) : (
    <Today initial={status} onChange={refresh} />
  );
}
