import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Current = { appName: string; title: string; appSecondsToday: number };
export type TrackingStatus = {
  paused: boolean;
  current: Current | null;
  todaySeconds: number;
  needsPermission: boolean;
  error: string | null;
  /** Süreli duraklatmanın bitişi. */
  pausedUntil: string | null;
  focus: FocusState | null;
};
export type FocusState = { startedAt: string; endsAt: string; minutes: number };
export type FocusTimer = { id: string; start: string; plannedEnd: string; end: string | null };
export type AppStatus = {
  platform: string;
  /** Pencere malzemesi: "vibrancy" (macOS), "mica" (Windows 11) ya da "none". */
  effect: string;
  accessibility: boolean;
  onboarded: boolean;
  autostart: boolean;
  theme: "system" | "light" | "dark";
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
export type DayBucket = {
  start: string;
  seconds: number;
  categories: Bucket[];
  focusScore: number;
  focusSeconds: number;
};
export type WorkBlock = {
  start: string;
  end: string;
  activeSeconds: number;
  categoryId: string | null;
  /** Bloğun en az yarısını kaplayan proje. */
  projectId: string | null;
  focus: boolean;
  switches: number;
  topApps: { appName: string; seconds: number }[];
};
export type FocusStats = {
  score: number;
  activeSeconds: number;
  focusSeconds: number;
  breakSeconds: number;
  switches: number;
  switchesPerHourX10: number;
  longestFocusSeconds: number;
  blocks: WorkBlock[];
};
export type Segment = {
  start: string;
  end: string;
  appName: string;
  title: string;
  categoryId: string | null;
};
export type WindowSpan = {
  start: string;
  end: string;
  appId: string;
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
  windows: WindowSpan[];
  tags: Tag[];
  focus: FocusStats;
  focusTimers: FocusTimer[];
};

export type ProjectSuggestion = { key: string; name: string; seconds: number; apps: string[] };
export type CategorySuggestion = {
  key: string;
  categoryId: string;
  field: RuleField;
  pattern: string;
  label: string;
  seconds: number;
};
export type Suggestions = { projects: ProjectSuggestion[]; categories: CategorySuggestion[] };

export type SearchResult = {
  totalSeconds: number;
  /** Dönemin her günü için süre (saniye). */
  days: number[];
  apps: { appId: string; appName: string; seconds: number }[];
  titles: { appName: string; title: string; seconds: number }[];
};

export type Trends = {
  /** Hafta başları (eskiden yeniye); son hafta sürüyor. */
  periods: string[];
  categories: TrendSeries[];
  projects: TrendSeries[];
};
export type TrendSeries = { id: string | null; seconds: number[] };

export type EntryKind = "Working" | "Online" | "F2F";
export type TimesheetEntry = {
  date: string;
  /** Yerel başlangıç saati, "HH:MM:SS". */
  start: string;
  hours: number;
  kind: EntryKind;
  details: string;
  party: string;
  projectId: string;
  division: string;
};
export type EntryView = TimesheetEntry & { id: string | null; exported: boolean };
export type TimesheetDay = {
  date: string;
  approved: boolean;
  entries: EntryView[];
  /** Projeye atanmamış takip edilen süre (saniye). */
  unassignedSeconds: number;
};
export type ProjectMapping = { projectId: string; division: string; party: string | null };
export type TimesheetConfig = {
  company: string;
  consultant: string;
  filePath: string | null;
  defaultParty: string;
  projects: ProjectMapping[];
  meetingApps: string[];
  /** Bir adam-günün saati. */
  dayHours: number;
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
  title_suffixes: string[];
};

export type Goals = {
  dailyHours: number;
  notifyGoal: boolean;
  /** `null` = mola hatırlatıcı kapalı. */
  breakAfterMinutes: number | null;
  /** Kategori başına günlük üst sınır (dakika). */
  limits: CategoryLimit[];
  /** Gün sonu özeti saati (gece yarısından dakika); `null` = kapalı. */
  daySummaryAt: number | null;
  /** Yeni haftanın ilk çalışmasında geçen haftanın özeti. */
  weeklySummary: boolean;
  /** Odak zamanlayıcısı sürerken dikkat dağıtıcı kategoriye geçince uyar. */
  focusGuard: boolean;
  /** Odak korumasının dikkat dağıtıcı saydığı kategori kimlikleri. */
  distracting: string[];
  /** Proje başına haftalık hedef (dakika). */
  projectGoals: ProjectGoal[];
};
export type ProjectGoal = { projectId: string; minutes: number };
export type CategoryLimit = { categoryId: string; minutes: number };

export type UpdateStatus = {
  current: string;
  available: string | null;
  notes: string | null;
  ready: boolean;
  checking: boolean;
  lastChecked: string | null;
  error: string | null;
};

export const api = {
  status: () => invoke<AppStatus>("get_status"),
  requestAccessibility: () => invoke<boolean>("request_accessibility"),
  openAccessibilitySettings: () => invoke<void>("open_accessibility_settings"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setTheme: (theme: "system" | "light" | "dark") => invoke<void>("set_theme", { theme }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  /** `null`: yarına kadar. */
  pauseFor: (minutes: number | null) => invoke<void>("pause_for", { minutes }),
  completeOnboarding: () => invoke<void>("complete_onboarding"),
  /** `until` verilirse rapor o anda kesilir (süren dönemin kıyası için). */
  report: (start: string, days: number, timeline: boolean, until?: string) =>
    invoke<Report>("get_report", { start, days, timeline, until: until ?? null }),
  appTitlesBetween: (appId: string, start: string, days: number) =>
    invoke<UsageTotal[]>("app_titles_between", { appId, start, days }),
  taxonomy: () => invoke<Taxonomy>("get_taxonomy"),
  saveTag: (tag: { id?: string; kind: TagKind; name: string; color: number }) => invoke<Tag>("save_tag", { tag }),
  deleteTag: (id: string) => invoke<void>("delete_tag", { id }),
  addRule: (tagId: string, field: RuleField, pattern: string) => invoke<void>("add_rule", { tagId, field, pattern }),
  deleteRule: (id: string) => invoke<void>("delete_rule", { id }),
  assignAppCategory: (appId: string, tagId: string | null) => invoke<void>("assign_app_category", { appId, tagId }),
  knownApps: () => invoke<UsageTotal[]>("known_apps"),
  suggestions: () => invoke<Suggestions>("get_suggestions"),
  /** `start` (YYYY-MM-DD) gününden itibaren `days` günde `query` geçen süre. */
  search: (query: string, start: string, days: number) => invoke<SearchResult>("search", { query, start, days }),
  trends: (weeks: number) => invoke<Trends>("get_trends", { weeks }),
  timesheetConfig: () => invoke<TimesheetConfig>("get_timesheet_config"),
  saveTimesheetConfig: (config: TimesheetConfig) => invoke<void>("save_timesheet_config", { config }),
  timesheetDays: (start: string, days: number) => invoke<TimesheetDay[]>("timesheet_days", { start, days }),
  approveTimesheetDay: (date: string) => invoke<void>("approve_timesheet_day", { date }),
  saveTimesheetEntry: (id: string | null, entry: TimesheetEntry) =>
    invoke<string>("save_timesheet_entry", { id, entry }),
  deleteTimesheetEntry: (id: string) => invoke<void>("delete_timesheet_entry", { id }),
  timesheetDetails: () => invoke<string[]>("timesheet_details"),
  pickTimesheetFile: () => invoke<string | null>("pick_timesheet_file"),
  importTimesheetTemplate: (path: string) =>
    invoke<{ config: TimesheetConfig; created: string[]; details: number }>("import_timesheet_template", { path }),
  exportTimesheet: (start: string, days: number) =>
    invoke<{ rows: number; filled: number; inserted: number; backup: string; path: string }>("export_timesheet", {
      start,
      days,
    }),
  /** Aramayla eşleşen oturumları İndirilenler'e CSV yazar; dosya yolunu döndürür. */
  exportSearch: (query: string, start: string, days: number) => invoke<string>("export_search", { query, start, days }),
  acceptProject: (name: string) => invoke<Tag>("accept_project_suggestion", { name }),
  acceptCategory: (s: CategorySuggestion) =>
    invoke<void>("accept_category_suggestion", { field: s.field, pattern: s.pattern, categoryId: s.categoryId }),
  dismissSuggestion: (key: string) => invoke<void>("dismiss_suggestion", { key }),
  /** Aralıktaki oturumlara elle kategori; `null` kurallara döndürür. */
  setRangeCategory: (start: string, end: string, categoryId: string | null) =>
    invoke<number>("set_range_category", { start, end, categoryId }),
  deleteRange: (start: string, end: string) => invoke<number>("delete_range", { start, end }),
  addManualEntry: (label: string, start: string, end: string, categoryId: string | null, projectId: string | null) =>
    invoke<void>("add_manual_entry", { label, start, end, categoryId, projectId }),
  /** Aralıktaki oturumlara elle proje; `null` kurallara döndürür. */
  setRangeProject: (start: string, end: string, projectId: string | null) =>
    invoke<number>("set_range_project", { start, end, projectId }),
  privacy: () => invoke<PrivacySettings>("get_privacy"),
  savePrivacy: (settings: PrivacySettings) => invoke<void>("save_privacy", { settings }),
  goals: () => invoke<Goals>("get_goals"),
  saveGoals: (goals: Goals) => invoke<void>("save_goals", { goals }),
  exportCsv: () => invoke<string>("export_csv"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncConfigure: (url: string, anonKey: string) => invoke<SyncStatus>("sync_configure", { url, anonKey }),
  syncSignIn: (email: string, password: string, signUp: boolean) =>
    invoke<SyncStatus>("sync_sign_in", { email, password, signUp }),
  syncSignOut: () => invoke<SyncStatus>("sync_sign_out"),
  syncDisconnect: () => invoke<SyncStatus>("sync_disconnect"),
  syncNow: () => invoke<void>("sync_now"),
  onSync: (cb: (s: SyncStatus) => void): Promise<UnlistenFn> => listen<SyncStatus>("sync", (e) => cb(e.payload)),
  diagnose: () => invoke<string>("diagnose"),
  startFocus: (minutes: number) => invoke<void>("start_focus", { minutes }),
  stopFocus: () => invoke<void>("stop_focus"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkUpdate: () => invoke<UpdateStatus>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
  onUpdate: (cb: (s: UpdateStatus) => void): Promise<UnlistenFn> =>
    listen<UpdateStatus>("update", (e) => cb(e.payload)),
  onStatus: (cb: (s: TrackingStatus) => void): Promise<UnlistenFn> =>
    listen<TrackingStatus>("status", (e) => cb(e.payload)),
};

/** "23dk", "1sa 5dk", "<1dk" — menü çubuğuyla aynı biçim. */
export function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return m === 0 ? (secs > 0 ? "<1dk" : "0dk") : `${m}dk`;
  return m === 0 ? `${h}sa` : `${h}sa ${m}dk`;
}
