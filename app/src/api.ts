import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Current = { appName: string; title: string; appSecondsToday: number };
export type TrackingStatus = {
  paused: boolean;
  current: Current | null;
  todaySeconds: number;
  needsPermission: boolean;
  error: string | null;
};
export type AppStatus = {
  platform: string;
  accessibility: boolean;
  onboarded: boolean;
  autostart: boolean;
  tracking: TrackingStatus;
};
export type UsageTotal = { key: string; label: string; seconds: number };

export const api = {
  status: () => invoke<AppStatus>("get_status"),
  requestAccessibility: () => invoke<boolean>("request_accessibility"),
  openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  completeOnboarding: () => invoke<void>("complete_onboarding"),
  todayApps: () => invoke<UsageTotal[]>("today_apps"),
  onStatus: (cb: (s: TrackingStatus) => void): Promise<UnlistenFn> =>
    listen<TrackingStatus>("status", (e) => cb(e.payload)),
};

/** "23dk", "1sa 5dk", "<1dk" — menü çubuğuyla aynı biçim. */
export function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return m === 0 ? "<1dk" : `${m}dk`;
  return m === 0 ? `${h}sa` : `${h}sa ${m}dk`;
}
