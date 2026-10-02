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

export type TagKind = "category" | "project";
export type Tag = { id: string; kind: TagKind; name: string; color: number };
export type RuleField = "app" | "title";
export type Rule = { id: string; tagId: string; field: RuleField; pattern: string };
export type Taxonomy = { tags: Tag[]; rules: Rule[] };

export type Bucket = { id: string | null; seconds: number };
export type AppBucket = { appId: string; appName: string; categoryId: string | null; seconds: number };
export type DayBucket = { start: string; seconds: number; categories: Bucket[] };
export type Segment = {
  start: string;
  end: string;
  appName: string;
  title: string;
  categoryId: string | null;
};
export type Report = {
  from: string;
  to: string;
  totalSeconds: number;
  categories: Bucket[];
  projects: Bucket[];
  apps: AppBucket[];
  days: DayBucket[];
  timeline: Segment[];
  tags: Tag[];
};

export type LastSync = { at: string; ok: boolean; message: string };
export type SyncStatus = {
  configured: boolean;
  url: string | null;
  email: string | null;
  last: LastSync | null;
};

/** Rust tarafında veritabanına bu adlarla kaydedildiği için snake_case. */
export type PrivacySettings = {
  paused: boolean;
  excluded_apps: string[];
  hidden_title_apps: string[];
  hide_private_windows: boolean;
};

export const api = {
  status: () => invoke<AppStatus>("get_status"),
  requestAccessibility: () => invoke<boolean>("request_accessibility"),
  openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  completeOnboarding: () => invoke<void>("complete_onboarding"),
  todayApps: () => invoke<UsageTotal[]>("today_apps"),
  appTitles: (appId: string) => invoke<UsageTotal[]>("app_titles", { appId }),
  report: (start: string, days: number, timeline: boolean) =>
    invoke<Report>("get_report", { start, days, timeline }),
  appTitlesBetween: (appId: string, start: string, days: number) =>
    invoke<UsageTotal[]>("app_titles_between", { appId, start, days }),
  taxonomy: () => invoke<Taxonomy>("get_taxonomy"),
  saveTag: (tag: { id?: string; kind: TagKind; name: string; color: number }) =>
    invoke<Tag>("save_tag", { tag }),
  deleteTag: (id: string) => invoke<void>("delete_tag", { id }),
  addRule: (tagId: string, field: RuleField, pattern: string) =>
    invoke<void>("add_rule", { tagId, field, pattern }),
  deleteRule: (id: string) => invoke<void>("delete_rule", { id }),
  assignAppCategory: (appId: string, tagId: string | null) =>
    invoke<void>("assign_app_category", { appId, tagId }),
  knownApps: () => invoke<UsageTotal[]>("known_apps"),
  privacy: () => invoke<PrivacySettings>("get_privacy"),
  savePrivacy: (settings: PrivacySettings) => invoke<void>("save_privacy", { settings }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncConfigure: (url: string, anonKey: string) =>
    invoke<SyncStatus>("sync_configure", { url, anonKey }),
  syncSignIn: (email: string, password: string, signUp: boolean) =>
    invoke<SyncStatus>("sync_sign_in", { email, password, signUp }),
  syncSignOut: () => invoke<SyncStatus>("sync_sign_out"),
  syncDisconnect: () => invoke<SyncStatus>("sync_disconnect"),
  syncNow: () => invoke<void>("sync_now"),
  onSync: (cb: (s: SyncStatus) => void): Promise<UnlistenFn> =>
    listen<SyncStatus>("sync", (e) => cb(e.payload)),
  diagnose: () => invoke<string>("diagnose"),
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
