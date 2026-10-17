/**
 * Uçtan uca testlerin sahte arka ucu: Tauri komutlarını bellekteki küçük bir veriyle yanıtlar.
 * Yanıtlar arayüzün kendi tipleriyle (`src/api.ts`) yazılır; arka ucun şekli değişince burası
 * da derlenmez. Tanımadığı komut `unmocked` listesine girer ve hata verir: test, gezdiği
 * sayfaların yalnızca bilinen komutları çağırdığını denetler.
 */
import type {
  AppStatus,
  Budgets,
  CalendarStatus,
  ClientReport,
  EntryView,
  Goals,
  KnownDevice,
  PrivacySettings,
  Report,
  SyncStatus,
  Tag,
  Taxonomy,
  TimesheetConfig,
  TimesheetDay,
  TrackingStatus,
  Unassigned,
  UpdateStatus,
  WindowSpan,
  WorkBlock,
} from "../src/api";

/** Testlerin "şimdi"si: 7 Ekim 2026 Çarşamba 14:00 (İstanbul). */
export const NOW = "2026-10-07T14:00:00+03:00";
export const TODAY = "2026-10-07";

const at = (hm: string, date = TODAY) => new Date(`${date}T${hm}:00+03:00`).toISOString();
const ms = (hm: string, date = TODAY) => Date.parse(at(hm, date));

export const PORTAL = "11111111-1111-4111-8111-111111111111";
export const TRUMORE = "22222222-2222-4222-8222-222222222222";
const DEV = "33333333-3333-4333-8333-333333333333";
const COMM = "44444444-4444-4444-8444-444444444444";
const CLIENT = "55555555-5555-4555-8555-555555555555";
export const SHEET = "66666666-6666-4666-8666-666666666666";

const tags: Tag[] = [
  { id: DEV, kind: "category", name: "Geliştirme", color: 1 },
  { id: COMM, kind: "category", name: "İletişim", color: 4 },
  { id: PORTAL, kind: "project", name: "Portal", color: 2, budgetDays: 10 },
  { id: TRUMORE, kind: "project", name: "Trumore", color: 5 },
];

const tracking: TrackingStatus = {
  paused: false,
  current: {
    appName: "Visual Studio Code",
    title: "portal — api.ts",
    appSecondsToday: 3 * 3600,
    project: { id: PORTAL, name: "Portal", color: 2, secondsToday: 4 * 3600 },
  },
  todaySeconds: 5 * 3600,
  needsPermission: false,
  error: null,
  pausedUntil: null,
};

const status: AppStatus = {
  platform: "macos",
  effect: "none",
  accessibility: true,
  onboarded: true,
  autostart: true,
  theme: "light",
  appIcon: "kum",
  tracking,
};

const span = (from: string, to: string, appId: string, appName: string, title: string, projectId: string | null) =>
  ({
    start: at(from),
    end: at(to),
    appId,
    appName,
    title,
    categoryId: DEV,
    projectId,
    domain: null,
  }) satisfies WindowSpan;

/** Bugünün işi: sabah Portal, öğleden sonra Trumore, arada projesiz Safari. */
const windows: WindowSpan[] = [
  span("09:00", "11:00", "com.microsoft.VSCode", "Visual Studio Code", "portal — api.ts", PORTAL),
  span("11:00", "11:30", "com.apple.Safari", "Safari", "Hacker News", null),
  span("12:30", "13:50", "com.microsoft.VSCode", "Visual Studio Code", "trumore — app.tsx", TRUMORE),
];

const block = (from: string, to: string, projectId: string | null, appName: string): WorkBlock => ({
  start: at(from),
  end: at(to),
  activeSeconds: (ms(to) - ms(from)) / 1000,
  categoryId: DEV,
  projectId,
  switches: 0,
  topApps: [{ appId: appName, appName, seconds: (ms(to) - ms(from)) / 1000 }],
  sent: projectId === PORTAL ? true : undefined,
});

function report(start: string, days: number): Report {
  const from = new Date(`${start}T00:00:00+03:00`);
  const to = new Date(+from + days * 86_400_000);
  const inRange = TODAY >= start && Date.parse(at("00:00")) < +to;
  const w = inRange ? windows : [];
  const total = w.reduce((s, x) => s + (Date.parse(x.end) - Date.parse(x.start)) / 1000, 0);
  return {
    from: from.toISOString(),
    to: to.toISOString(),
    totalSeconds: total,
    categories: inRange ? [{ id: DEV, seconds: total }] : [],
    projects: inRange
      ? [
          { id: PORTAL, seconds: 7200 },
          { id: TRUMORE, seconds: 4800 },
          { id: null, seconds: 1800 },
        ]
      : [],
    apps: inRange
      ? [
          { appId: "com.microsoft.VSCode", appName: "Visual Studio Code", categoryId: DEV, seconds: 12_000 },
          { appId: "com.apple.Safari", appName: "Safari", categoryId: DEV, seconds: 1800 },
        ]
      : [],
    days: Array.from({ length: days }, (_, i) => {
      const d = new Date(+from + i * 86_400_000);
      const today = d.toISOString().slice(0, 10) === new Date(at("00:00")).toISOString().slice(0, 10);
      return {
        start: d.toISOString(),
        seconds: today ? total : 0,
        categories: today ? [{ id: DEV, seconds: total }] : [],
      };
    }),
    timeline: [],
    windows: w,
    tags,
    work: {
      activeSeconds: total,
      breakSeconds: 3600,
      switches: 2,
      switchesPerHourX10: 5,
      blocks: inRange
        ? [
            block("09:00", "11:00", PORTAL, "Visual Studio Code"),
            block("11:00", "11:30", null, "Safari"),
            block("12:30", "13:50", TRUMORE, "Visual Studio Code"),
          ]
        : [],
    },
    idleSeconds: 0,
    idle: [],
    devices: [],
  };
}

const entry = (over: Partial<EntryView>): EntryView => ({
  id: null,
  key: "live",
  exported: false,
  stale: null,
  openUntil: 0,
  date: TODAY,
  start: "09:00:00",
  hours: 2,
  actualHours: 2,
  kind: "Working",
  details: "API uçları",
  party: "ADBA",
  projectId: PORTAL,
  division: "Portal",
  coverage: [[ms("09:00"), ms("11:00")]],
  ...over,
});

/** Bugün Portal'ın kaydedilmiş sabah satırı ve süren (henüz gönderilmeyecek) öğleden sonra satırı. */
function timesheetDays(start: string, days: number): TimesheetDay[] {
  return Array.from({ length: days }, (_, i) => {
    const date = new Date(Date.parse(`${start}T12:00:00+03:00`) + i * 86_400_000).toISOString().slice(0, 10);
    const entries =
      date === TODAY
        ? [
            entry({ id: "saved-1", key: "saved-1", openUntil: ms("11:00") }),
            entry({
              key: "live-2",
              start: "13:00:00",
              hours: 1,
              actualHours: 0.83,
              details: "",
              coverage: [[ms("13:00"), ms("13:50")]],
              // Çekirdeğe göre iş sürüyor: son işinden sonra 15 dk dolmadı.
              openUntil: ms("14:05"),
            }),
          ]
        : [];
    return { date, entries, hidden: 0, unassignedSeconds: date === TODAY ? 1800 : 0, meetings: [] };
  });
}

const timesheetConfig: TimesheetConfig = {
  timesheets: [
    {
      id: SHEET,
      company: "ADBA",
      consultant: "Test Kullanıcı",
      filePath: "/tmp/zaman.xlsx",
      sheetUrl: null,
      sheetLink: null,
      defaultParty: "ADBA",
      projects: [{ projectId: PORTAL, division: "Portal", party: null }],
      divisions: ["Portal"],
    },
  ],
  sheetToken: "",
  meetingApps: [],
  dayHours: 8,
};

const unassigned: Unassigned = {
  totalSeconds: 1800,
  groups: [
    {
      key: "app:com.apple.Safari",
      kind: "app",
      label: "Safari",
      appName: "Safari",
      pattern: "com.apple.Safari",
      seconds: 1800,
      items: [{ title: "Hacker News", seconds: 1800, word: null }],
      more: 0,
      likelyProject: PORTAL,
    },
  ],
  idle: [],
  idleSeconds: 0,
};

const devices: KnownDevice[] = [
  {
    id: "d-1",
    name: "Mac Studio",
    os: "macos",
    model: "Mac Studio",
    icon: "",
    current: true,
    version: "0.9.74",
    lastSeen: at("13:50"),
    outdated: false,
  },
  {
    id: "d-2",
    name: "MacBook Air",
    os: "macos",
    model: "MacBook Air",
    icon: "",
    current: false,
    version: "0.9.70",
    lastSeen: at("09:00", "2026-10-06"),
    outdated: true,
  },
];

const sync: SyncStatus = {
  configured: true,
  url: "https://ornek.supabase.co",
  schema: null,
  email: "test@example.com",
  last: {
    at: at("13:55"),
    ok: true,
    message: "3 gönderildi, 2 alındı. MacBook Air eski sürümde: o bilgisayarda Kum'u güncelle",
  },
};

const privacy: PrivacySettings = {
  paused: false,
  excluded_apps: [],
  excluded_urls: [],
  hidden_title_apps: [],
  hide_private_windows: true,
  title_suffixes: [],
  record_idle: true,
  idle_max_minutes: 180,
  count_watching: true,
  detect_calls: true,
};

const goals: Goals = {
  dailyHours: 8,
  notifyGoal: true,
  breakAfterMinutes: null,
  limits: [],
  daySummaryAt: 18 * 60,
  weeklySummary: true,
  projectGoals: [],
  exportReminderAt: 17 * 60,
};

const budgets: Budgets = {
  dayHours: 8,
  projects: [{ id: PORTAL, budgetDays: 10, budgetSeconds: 10 * 8 * 3600, usedSeconds: 9 * 8 * 3600, level: "near" }],
  clients: [],
};

const taxonomy: Taxonomy = {
  tags,
  rules: [{ id: "r-1", tagId: PORTAL, field: "title", pattern: "portal" }],
  clients: [{ id: CLIENT, name: "ADBA" }],
  projectClients: { [PORTAL]: CLIENT, [TRUMORE]: CLIENT },
};

const update: UpdateStatus = {
  current: "0.9.74",
  available: null,
  notes: null,
  ready: false,
  checking: false,
  lastChecked: at("13:00"),
  error: null,
};

const calendar: CalendarStatus = { url: null, events: 0, last: null, ignored: 0 };

type Args = Record<string, unknown>;
type Handler = (args: Args) => unknown;

/** Komut adı → yanıt. Yalnızca okuyan ya da etkisi testte görülmeyen komutlar. */
const handlers: Record<string, Handler> = {
  get_status: () => status,
  log_client: () => null,
  get_report: (a) => report(a.start as string, a.days as number),
  get_taxonomy: () => taxonomy,
  get_budgets: () => budgets,
  get_suggestions: () => ({ projects: [], categories: [] }),
  rule_suggestions: () => [],
  get_unassigned: () => unassigned,
  ignored_unassigned: () => [],
  assign_unassigned: () => ({ changed: 1, undo: 7 }),
  label_undo: () => null,
  undo_history: () => [],
  pending_timesheet_days: () => [],
  get_timesheet_config: () => timesheetConfig,
  timesheet_days: (a) => timesheetDays(a.start as string, a.days as number),
  timesheet_details: () => [],
  closed_days: () => [],
  sheet_rows: () => ({ rows: [], missing: [], synced: 0 }),
  calendar_meetings: () => [],
  calendar_status: () => calendar,
  list_devices: () => devices,
  sync_status: () => sync,
  update_status: () => update,
  get_goals: () => goals,
  get_privacy: () => privacy,
  default_excluded_urls: () => [],
  shortcut_status: () => ({ enabled: true, label: "⌃⌥⌘K", error: null }),
  app_icon: () => null,
  site_icon: () => null,
  known_apps: () => [],
  get_trends: () => ({ periods: [], categories: [], projects: [] }),
  google_status: () => ({ connected: false, email: null, clientId: "", hasSecret: false }),
  get_ai_settings: () => ({ enabled: false, hasKey: false, keyHint: null, model: "claude" }),
  backup_status: () => ({ dir: "/tmp/yedek", last: null, damage: null, files: [] }),
  client_report: (a) => clientReport(a.month as string),
  preview_rule: () => ({
    matchedSeconds: 1800,
    gainedSeconds: 1800,
    takenSeconds: 0,
    alreadySeconds: 0,
    blockedSeconds: 0,
    takenFrom: [],
    samples: [{ appName: "Safari", title: "Hacker News", seconds: 1800 }],
  }),
};

function clientReport(month: string): ClientReport {
  const days = Array.from({ length: 31 }, (_, i) => `${month}-${String(i + 1).padStart(2, "0")}`);
  const hours = days.map((d) => (d === TODAY ? 3 : 0));
  return {
    days,
    source: "timesheet",
    timesheetAvailable: true,
    rows: [{ projectId: PORTAL, project: "Portal", color: 2, client: "ADBA", hours, total: 3 }],
    dayTotals: hours,
    total: 3,
  };
}

export type Call = { cmd: string; args: Args };

/** Sayfadaki sahte arka uç: çağrılar ve tanınmayan komutlar testten okunur. */
export type FakeBackend = {
  calls: Call[];
  unmocked: string[];
  handle: (cmd: string, args?: Args) => unknown;
};

export function createBackend(): FakeBackend {
  const backend: FakeBackend = {
    calls: [],
    unmocked: [],
    handle(cmd, args = {}) {
      backend.calls.push({ cmd, args });
      // Olay dinleyicileri (`listen`) mockIPC'nin olay taklidine kalır.
      if (cmd.startsWith("plugin:")) return null;
      const h = handlers[cmd];
      if (!h) {
        backend.unmocked.push(cmd);
        throw new Error(`sahte arka uç: ${cmd} tanımlı değil`);
      }
      return structuredClone(h(args));
    },
  };
  return backend;
}
