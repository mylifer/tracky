import { invoke as tauriInvoke, type InvokeArgs } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Arayüzden günlüğe (`kum.log`) yazar; yazılamazsa sessizce geçer. */
export function logClient(level: "error" | "info", message: string) {
  tauriInvoke("log_client", { level, message }).catch(() => {});
}

/** Başarısız komutlar hata günlüğüne de yazılır; hata çağırana aynen döner. */
async function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await tauriInvoke<T>(cmd, args);
  } catch (e) {
    logClient("error", `${cmd}: ${e instanceof Error ? e.message : String(e)}`);
    throw e;
  }
}

export type Current = {
  appName: string;
  title: string;
  appSecondsToday: number;
  /** Şu anki oturumun projesi (kurala ya da elle atamaya göre) ve bugünkü süresi. */
  project: { id: string; name: string; color: number; secondsToday: number } | null;
};
export type TrackingStatus = {
  paused: boolean;
  current: Current | null;
  todaySeconds: number;
  needsPermission: boolean;
  error: string | null;
  /** Süreli duraklatmanın bitişi. */
  pausedUntil: string | null;
};
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
/** Oturuma elle verilen "proje yok" (kurala uysa da projeye sayılmaz); çekirdekteki `NO_PROJECT`. */
export const NO_PROJECT = "00000000-0000-0000-0000-000000000000";
export type Tag = {
  id: string;
  kind: TagKind;
  name: string;
  color: number;
  /** Arşivdeki proje: seçicilerde görünmez, yeni süre toplamaz (yalnızca `taxonomy` doldurur). */
  archived?: boolean;
  /** Sözleşme bütçesi (adam-gün; yalnızca `taxonomy` doldurur). */
  budgetDays?: number | null;
};
export type RuleField = "app" | "title" | "domain";
export type Rule = { id: string; tagId: string; field: RuleField; pattern: string };
export type Client = { id: string; name: string; budgetDays?: number | null };
/** Proje ya da müşterinin bütçesi ve bugüne kadar harcanan süre. */
export type BudgetUsage = { id: string; budgetDays: number; budgetSeconds: number; usedSeconds: number };
export type Budgets = {
  /** Bir adam-günün saati (zaman çizelgesi ayarı; yoksa 8). */
  dayHours: number;
  projects: BudgetUsage[];
  clients: BudgetUsage[];
};
export type Taxonomy = {
  tags: Tag[];
  rules: Rule[];
  clients: Client[];
  /** Proje → müşteri. */
  projectClients: Record<string, string>;
};

export type Bucket = { id: string | null; seconds: number };
export type AppBucket = { appId: string; appName: string; categoryId: string | null; seconds: number };
export type DayBucket = {
  start: string;
  seconds: number;
  categories: Bucket[];
};
export type WorkBlock = {
  start: string;
  end: string;
  activeSeconds: number;
  categoryId: string | null;
  /** Bloğun en az yarısını kaplayan proje. */
  projectId: string | null;
  switches: number;
  topApps: { appId: string; appName: string; seconds: number }[];
  /** Bloktaki süre bilgisayar başına; yalnızca aralıkta birden çok bilgisayar varsa. */
  devices?: BlockDevice[];
};
export type BlockDevice = { id: string; name: string; os: string; model: string; seconds: number };
/** Aralıkta çalışılan bilgisayar (`Report.devices`). */
export type DeviceTotal = { id: string; name: string; os: string; model: string; seconds: number; current: boolean };
/** Kayıtlı bilgisayar (Ayarlar → Eşitleme). */
/** `model`: "Mac Studio", "MacBook Pro" ...; bilinmiyorsa boş. */
export type KnownDevice = { id: string; name: string; os: string; model: string; current: boolean };
/** Çalışma blokları ve molalar. */
export type WorkStats = {
  activeSeconds: number;
  breakSeconds: number;
  switches: number;
  switchesPerHourX10: number;
  blocks: WorkBlock[];
};
export type Segment = {
  start: string;
  end: string;
  appName: string;
  title: string;
  categoryId: string | null;
};
/** Aralık düzenlemesi yalnızca bu uygulamalara (ve doluysa bu pencere başlıklarına) uygulanır. */
export type EditScope = { appIds: string[]; titles: string[] | null };

export type WindowSpan = {
  start: string;
  end: string;
  appId: string;
  appName: string;
  title: string;
  categoryId: string | null;
  /** Pencerenin (kurala ya da elle atamaya göre) projesi. */
  projectId: string | null;
  /** Tarayıcıdaysa sitenin alan adı. */
  domain: string | null;
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
  work: WorkStats;
  /** Bilgisayardan uzakta geçen, bir işe atanmamış süre; çalışma toplamına girmez. */
  idleSeconds: number;
  /** Atanmamış boşta aralıklar (takvimde "Boşta"). */
  idle: IdleSpan[];
  /** Aralıkta çalışılan bilgisayarlar (filtresiz); yalnızca birden çok bilgisayar varsa dolu. */
  devices: DeviceTotal[];
};
export type IdleSpan = { start: string; end: string };

/** Proje profili (`get_project_stats`). */
export type ProjectStats = {
  totalSeconds: number;
  /** Gün başına süre, istenen ilk günden itibaren. */
  days: number[];
  /** Yerel saate göre günün 24 saatine dağılım. */
  hours: number[];
  apps: { appId: string; appName: string; seconds: number }[];
  titles: { appName: string; title: string; seconds: number }[];
  categories: Bucket[];
  /** Çoğunluğu bu projede geçen çalışma blokları. */
  blocks: number;
  /** Bu blokların ortalama etkin süresi. */
  focusSeconds: number;
  switchesPerHourX10: number;
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
  /** Firmaya yazılan saat (çeyrek saate yuvarlanmış). */
  hours: number;
  /** Takip edilen gerçek süre (saat); elle eklenen satırda yok. */
  actualHours?: number | null;
  kind: EntryKind;
  details: string;
  party: string;
  projectId: string;
  division: string;
  /**
   * Satırın kapsadığı takip aralıkları (`[başlangıç, bitiş]`, unix ms); kaydedilmiş satırların
   * aralıkları yeniden önerilmez. Boş: elle eklenen satır, `null`: önceki sürümün kaydı.
   */
  coverage?: [number, number][] | null;
};
export type EntryView = TimesheetEntry & {
  /** Kaydedilmiş satırın kimliği; takipten gelen (henüz kaydedilmemiş, canlı) satırda `null`. */
  id: string | null;
  /** Sayfadaki kararlı anahtar: canlı satır kaydedilince de aynı kalır. */
  key: string;
  exported: boolean;
  /** Takipte değişti: işin bir kısmı raporda başka projeye alınmış; projede kalan süre (saat). */
  stale: number | null;
};
/** Firmanın dosyasındaki (Excel ya da Sheets) bir kayıt satırı. */
export type FileRow = {
  /** Dosyadaki satır numarası (yazarken ipucu). */
  row: number;
  date: string;
  /** "HH:MM:SS"; hücre boşsa ya da saat değilse `null`. */
  start: string | null;
  hours: number | null;
  kind: string;
  details: string;
  party: string;
  division: string;
  consultant: string;
};
/** Dosyadaki satır: Kum aktardıysa kaydının kimliğiyle. */
export type SheetRowView = FileRow & {
  entryId: string | null;
  /** Yalnızca arayüzde: satırın ekrandaki kalıcı kimliği (React anahtarı; arka uca gitmez). */
  uid?: number;
};
export type SheetRows = {
  rows: SheetRowView[];
  /** Bu çizelgeye aktarılmış olup dosyada bulunamayan kayıtlar. */
  missing: string[];
  /** Dosyada değiştirildiği için Kum'da da güncellenen kayıt sayısı. */
  synced: number;
};
/** Takvimden (Outlook) bir toplantı. */
export type Meeting = {
  /** Serinin kimliği: tekrarlayan toplantının hepsinde aynı. */
  uid: string;
  start: string;
  end: string;
  subject: string;
  location: string;
  online: boolean;
};
/** Projesi belli olmayan toplantı için önerilen proje ve kısa gerekçe ("katılımcılar @acme.com"). */
export type MeetingSuggestion = { projectId: string; reason: string };
/** Projesi belli olmayan toplantı ve (emin olunursa) önerilen proje. */
export type UnassignedMeeting = Meeting & { suggestion?: MeetingSuggestion | null };
/** Gün takvimindeki toplantı: serinin projesi (elle ya da kuraldan) ile. */
export type CalendarMeeting = Meeting & {
  projectId: string | null;
  /** Seri zaman çizelgesine alınmıyor. */
  ignored: boolean;
  /** Projesi belli değilse önerilen proje. */
  suggestion?: MeetingSuggestion | null;
};
export type TimesheetDay = {
  date: string;
  /** Kaydedilmiş satırlar ve canlı öneriler, başlangıca göre. */
  entries: EntryView[];
  /** Gizlenen (silinen) satır sayısı. */
  hidden: number;
  /** Projeye atanmamış takip edilen süre (saniye). */
  unassignedSeconds: number;
  /** Hiçbir projeye düşmeyen takvim toplantıları. */
  meetings: UnassignedMeeting[];
};
export type CalendarStatus = {
  url: string | null;
  events: number;
  last: { at: string; ok: boolean; message: string } | null;
  /** Yoksayılan toplantı serisi sayısı. */
  ignored: number;
};
/** Google hesabı bağlantısı: bağlıysa tablolar Sheets API ile doğrudan okunup yazılır. */
export type GoogleStatus = {
  connected: boolean;
  email: string | null;
  clientId: string;
  /** Gizli anahtar kayıtlı (kendisi gönderilmez). */
  hasSecret: boolean;
};
export type Imported = {
  config: TimesheetConfig;
  /** İçe aktarılan (yeni ya da güncellenen) zaman çizelgesi. */
  timesheetId: string;
  created: string[];
  details: number;
};
/** Birleştirilen satır ve geri almak için silinen kaydedilmiş satırlar. */
export type Merged = { id: string; removed: { id: string; entry: TimesheetEntry }[] };
/** Gönderilecek, birleştirilecek satır: kaydedilmişse kimliğiyle. */
export type RowRef = { id: string | null; entry: TimesheetEntry };
export type Exported = {
  rows: number;
  filled: number;
  inserted: number;
  skipped: number;
  backup: string | null;
  /** Excel dosyası ya da Sheets sayfası. */
  target: string;
  sheets: boolean;
};
export type ProjectMapping = {
  projectId: string;
  /** Projenin satırlarına yazılan birim; boşsa proje adı. */
  division: string;
  party: string | null;
  /** Hazır açıklama: önerinin başlıklardan açıklaması çıkmazsa yazılır. */
  defaultDetails?: string | null;
};
/** Bir firmanın zaman çizelgesi: kayıtların yazıldığı dosya ve yalnızca oraya giden projeler. */
export type Timesheet = {
  id: string;
  company: string;
  consultant: string;
  filePath: string | null;
  /** Apps Script web uygulaması (…/exec); doluysa kayıtlar Google Sheets'e gider. */
  sheetUrl: string | null;
  /** Tablonun docs.google.com bağlantısı. */
  sheetLink: string | null;
  defaultParty: string;
  /** Bu çizelgeye giden projeler; bir proje yalnızca bir çizelgede olur. */
  projects: ProjectMapping[];
  /** Dosyadaki birimler (şablondan): satırın birimi bunlardan seçilebilir. */
  divisions: string[];
};
export type TimesheetConfig = {
  timesheets: Timesheet[];
  sheetToken: string;
  meetingApps: string[];
  /** Bir adam-günün saati. */
  dayHours: number;
};

/** Yapay zekâyla açıklama yazma ayarı; anahtarın yalnızca son dört karakteri gelir. */
export type AiStatus = { enabled: boolean; hasKey: boolean; keyHint: string | null; model: string };
/** Yapay zekânın bir satıra (sayfadaki anahtarıyla) yazdığı açıklama (henüz kaydedilmedi). */
export type AiChange = { key: string; details: string };

export type BackupFile = { name: string; path: string; at: string; bytes: number };
export type BackupStatus = {
  dir: string;
  last: string | null;
  /** Veritabanında bozulma bulunduysa açıklaması (o sürece yedek alınmaz). */
  damage: string | null;
  files: BackupFile[];
};
export type PickedBackup = { path: string; sessions: number; lastActivity: string | null };

export type LastSync = { at: string; ok: boolean; message: string };
export type SyncStatus = {
  configured: boolean;
  url: string | null;
  /** Tabloların şeması (paylaşılan projede); boşsa public. */
  schema: string | null;
  email: string | null;
  last: LastSync | null;
};

/** Rust tarafında veritabanına bu adlarla kaydedildiği için snake_case. */
export type PrivacySettings = {
  paused: boolean;
  excluded_apps: string[];
  /** Tarayıcıda bu adreslerde geçen süre kaydedilmez (`site.com` alt alan adlarını da kapsar). */
  excluded_urls: string[];
  hidden_title_apps: string[];
  hide_private_windows: boolean;
  title_suffixes: string[];
  /** Bilgisayardan uzakta geçen süre takvimde "Boşta" olarak kaydedilir. */
  record_idle: boolean;
  /** Bundan uzun boşluklar (gece gibi) kaydedilmez (dakika). */
  idle_max_minutes: number;
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
  /** Proje başına haftalık hedef (dakika). */
  projectGoals: ProjectGoal[];
  /** Cuma bu saatte aktarılmamış günler hatırlatılır (gece yarısından dakika); `null` = kapalı. */
  exportReminderAt: number | null;
};
export type ProjectGoal = { projectId: string; minutes: number };
export type CategoryLimit = { categoryId: string; minutes: number };

/**
 * Geri alınabilir düzenleme: değişen kayıt sayısı ve geri alma numarası (`api.undo`). Elle
 * atamadan sonra atanan süre bir alışkanlığa dönüştüyse önerilen kural da gelir.
 */
export type Edited = { changed: number; undo: number; suggestion?: RuleSuggestion };

/** Değişiklik geçmişindeki bir satır; `blocked`: aynı süreye dokunan daha yeni değişiklik var. */
export type HistoryItem = { id: number; at: string; label: string | null; blocked: boolean };
export type ShortcutStatus = { enabled: boolean; label: string; error: string | null };

/** Elle atamalardan öğrenilen kural önerisi. */
export type RuleSuggestion = {
  /** Yoksayma anahtarı. */
  key: string;
  projectId: string;
  projectName: string;
  field: RuleField;
  pattern: string;
  /** İş anahtarı öneki (`LOY-`), başlıktaki proje adı ya da site. */
  source: "issueKey" | "titleWord" | "site";
  /** Ayrı elle atama sayısı. */
  assignments: number;
  days: number;
  /** Desene uyan, elle bu projeye atanmış süre. */
  manualSeconds: number;
  preview: RulePreview;
};

export type UnassignedItem = { title: string; seconds: number; word: string | null };
export type UnassignedGroup = {
  /** `site:alan` ya da `app:kimlik`. */
  key: string;
  kind: "site" | "app";
  label: string;
  appName: string;
  /** Kural deseni: alan adı ya da uygulama kimliği. */
  pattern: string;
  seconds: number;
  items: UnassignedItem[];
  more: number;
  /** Önünde/arkasında çalışılan proje. */
  likelyProject: string | null;
};
export type Unassigned = {
  totalSeconds: number;
  groups: UnassignedGroup[];
  idle: { start: string; end: string; seconds: number }[];
  idleSeconds: number;
};
export type RulePreview = {
  matchedSeconds: number;
  gainedSeconds: number;
  takenSeconds: number;
  alreadySeconds: number;
  blockedSeconds: number;
  takenFrom: { id: string; seconds: number }[];
  samples: { appName: string; title: string; seconds: number }[];
};

export type UpdateStatus = {
  current: string;
  available: string | null;
  notes: string | null;
  ready: boolean;
  checking: boolean;
  lastChecked: string | null;
  error: string | null;
};

/** Müşteri raporunun kaynağı: zaman çizelgesi satırları ya da takip edilen süre. */
export type ReportSource = "timesheet" | "tracked";
export type ClientReportRow = {
  /** Projesi bilinmeyen zaman çizelgesi kaydında boş. */
  projectId: string;
  project: string;
  color: number;
  client: string | null;
  /** Gün başına saat, `days` sırasıyla. */
  hours: number[];
  total: number;
};
export type ClientReport = {
  days: string[];
  source: ReportSource;
  timesheetAvailable: boolean;
  rows: ClientReportRow[];
  dayTotals: number[];
  total: number;
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
  /** `device` verilirse yalnızca o bilgisayarın kaydettiği süre. */
  report: (start: string, days: number, timeline: boolean, until?: string, device?: string | null) =>
    invoke<Report>("get_report", { start, days, timeline, until: until ?? null, device: device ?? null }),
  listDevices: () => invoke<KnownDevice[]>("list_devices"),
  renameDevice: (id: string, name: string) => invoke<KnownDevice[]>("rename_device", { id, name }),
  /** Uygulamanın simgesi (`data:` adresi); bu bilgisayarda yoksa `null`. */
  appIcon: (appId: string) => invoke<string | null>("app_icon", { appId }),
  /** Sitenin simgesi (favicon, `data:` adresi); bulunamazsa `null`. */
  siteIcon: (domain: string) => invoke<string | null>("site_icon", { domain }),
  appTitlesBetween: (appId: string, start: string, days: number) =>
    invoke<UsageTotal[]>("app_titles_between", { appId, start, days }),
  taxonomy: () => invoke<Taxonomy>("get_taxonomy"),
  saveClient: (id: string | null, name: string) => invoke<Client>("save_client", { id, name }),
  deleteClient: (id: string) => invoke<void>("delete_client", { id }),
  /** Projeyi müşteriye bağla; `null` müşterisiz yapar. */
  setProjectClient: (projectId: string, clientId: string | null) =>
    invoke<void>("set_project_client", { projectId, clientId }),
  saveTag: (tag: { id?: string; kind: TagKind; name: string; color: number }) => invoke<Tag>("save_tag", { tag }),
  /** Projeyi arşivler; geri alma numarasını döndürür. */
  archiveProject: (id: string) => invoke<number>("archive_project", { id }),
  /** Projeyi arşivden çıkarır; geri alma numarasını döndürür. */
  unarchiveProject: (id: string) => invoke<number>("unarchive_project", { id }),
  /** Sözleşme bütçesi (adam-gün); `null` kaldırır. */
  setProjectBudget: (id: string, days: number | null) => invoke<void>("set_project_budget", { id, days }),
  setClientBudget: (id: string, days: number | null) => invoke<void>("set_client_budget", { id, days }),
  budgets: () => invoke<Budgets>("get_budgets"),
  /** Geri alma numarasını döndürür. */
  deleteTag: (id: string) => invoke<number>("delete_tag", { id }),
  addRule: (tagId: string, field: RuleField, pattern: string) => invoke<number>("add_rule", { tagId, field, pattern }),
  deleteRule: (id: string) => invoke<number>("delete_rule", { id }),
  /** Kural eklenseydi son 30 günde ne değişirdi? */
  previewRule: (tagId: string, field: RuleField, pattern: string) =>
    invoke<RulePreview>("preview_rule", { tagId, field, pattern }),
  /** Son 30 günün elle atamalarından öğrenilen kural önerileri. */
  ruleSuggestions: () => invoke<RuleSuggestion[]>("rule_suggestions"),
  /** Kural önerisini bir daha gösterme. */
  dismissRuleSuggestion: (key: string) => invoke<void>("dismiss_rule_suggestion", { key }),
  /** Düzenlemeyi geri alır. */
  undo: (id: number) => invoke<void>("undo", { id }),
  /** Geri alma kaydına bildirimdeki iletiyi ekler (değişiklik geçmişinde görünür). */
  labelUndo: (id: number, label: string) => invoke<void>("label_undo", { id, label }),
  /** Bu açılışta yapılan, hâlâ geri alınabilecek değişiklikler (en yeni önce). */
  undoHistory: () => invoke<HistoryItem[]>("undo_history"),
  /** "Son süreyi ata" için her yerden kısayol. */
  shortcutStatus: () => invoke<ShortcutStatus>("shortcut_status"),
  setShortcutEnabled: (enabled: boolean) => invoke<ShortcutStatus>("set_shortcut_enabled", { enabled }),
  unassigned: (start: string, days: number) => invoke<Unassigned>("get_unassigned", { start, days }),
  /** Grubu (ya da başlığı) projeye atar; `rule` verilirse önce kural eklenir. */
  assignUnassigned: (
    start: string,
    days: number,
    key: string,
    title: string | null,
    projectId: string | null,
    rule: [RuleField, string] | null,
  ) => invoke<Edited>("assign_unassigned", { start, days, key, title, projectId, rule }),
  /** Takvim bloğundaki bir pencereyi (`from`–`to` içinde) projeye atar; `null` kurallara bırakır. */
  assignWindow: (from: string, to: string, appId: string, title: string, projectId: string | null) =>
    invoke<Edited>("assign_window", { from, to, appId, title, projectId }),
  /** `month` ayının proje × gün saatleri; `client` `null` ise tüm müşteriler, `source` `null` ise kendisi seçer. */
  clientReport: (month: string, client: string | null, source: ReportSource | null) =>
    invoke<ClientReport>("client_report", { month, client, source }),
  /** Raporu seçilen Excel dosyasına yazar; vazgeçilirse `null`. */
  exportClientReport: (month: string, client: string | null, source: ReportSource | null) =>
    invoke<string | null>("export_client_report", { month, client, source }),
  ignoreUnassigned: (key: string, ignored: boolean) => invoke<void>("ignore_unassigned", { key, ignored }),
  ignoredUnassigned: () => invoke<string[]>("ignored_unassigned"),
  /** Bu hafta aktarılmamış işi olan günler (YYYY-MM-DD). */
  pendingTimesheetDays: () => invoke<string[]>("pending_timesheet_days"),
  assignAppCategory: (appId: string, tagId: string | null) => invoke<void>("assign_app_category", { appId, tagId }),
  knownApps: () => invoke<UsageTotal[]>("known_apps"),
  suggestions: () => invoke<Suggestions>("get_suggestions"),
  /** `start` (YYYY-MM-DD) gününden itibaren `days` günde `query` geçen süre. */
  search: (query: string, start: string, days: number) => invoke<SearchResult>("search", { query, start, days }),
  trends: (weeks: number) => invoke<Trends>("get_trends", { weeks }),
  projectStats: (id: string, start: string, days: number) =>
    invoke<ProjectStats>("get_project_stats", { id, start, days }),
  timesheetConfig: () => invoke<TimesheetConfig>("get_timesheet_config"),
  saveTimesheetConfig: (config: TimesheetConfig) => invoke<void>("save_timesheet_config", { config }),
  timesheetDays: (timesheetId: string, start: string, days: number) =>
    invoke<TimesheetDay[]>("timesheet_days", { timesheetId, start, days }),
  /** Satırı kaydeder: kaydedilmişi günceller, canlı satırı (`id` yok) ya da elle eklenen satırı ekler; kimliği döner. */
  /** Aktarılmış satır dosyadaki satırıyla birlikte değişir (`sheetRow`: dosyadaki satır numarası). */
  saveTimesheetEntry: (id: string | null, entry: TimesheetEntry, sheetRow?: number | null) =>
    invoke<string>("save_timesheet_entry", { id, entry: plainEntry(entry), sheetRow: sheetRow ?? null }),
  /** Satırı gizler (siler); canlı satır gizlenmiş olarak kaydedilir. Kimliği döner. */
  /** Aktarılmış satır dosyadan da kaldırılır. */
  dismissTimesheetEntry: (id: string | null, entry: TimesheetEntry, sheetRow?: number | null) =>
    invoke<string>("dismiss_timesheet_entry", { id, entry: plainEntry(entry), sheetRow: sheetRow ?? null }),
  /** Çizelgenin dosyasındaki dönemin satırları (Kum'un aktardıkları kayıtlarıyla eşlenmiş). */
  sheetRows: (timesheetId: string, start: string, days: number) =>
    invoke<SheetRows>("sheet_rows", { timesheetId, start, days }),
  /** Dosyada Kum dışında girilmiş satırı değiştirir (tarih değiştiyse satır taşınır); yazılan satırın numarası. */
  saveSheetRow: (timesheetId: string, expect: FileRow, row: FileRow) =>
    invoke<number>("save_sheet_row", { timesheetId, expect: plainFileRow(expect), row: plainFileRow(row) }),
  /** Silinen dosya satırını gününe geri ekler; yazılan satırın numarası. */
  restoreSheetRow: (timesheetId: string, row: FileRow) =>
    invoke<number>("restore_sheet_row", { timesheetId, row: plainFileRow(row) }),
  deleteSheetRow: (timesheetId: string, expect: FileRow) =>
    invoke<void>("delete_sheet_row", { timesheetId, expect: plainFileRow(expect) }),
  undismissTimesheetEntries: (ids: string[]) => invoke<void>("undismiss_timesheet_entries", { ids }),
  /** Günün gizlenen satırlarını geri getirir. */
  restoreHiddenEntries: (timesheetId: string, date: string) =>
    invoke<string[]>("restore_hidden_entries", { timesheetId, date }),
  /** Satırı tamamen siler (gizlenen canlı satır yeniden öneri olur). */
  deleteTimesheetEntry: (id: string) => invoke<void>("delete_timesheet_entry", { id }),
  mergeTimesheetEntries: (rows: RowRef[]) => invoke<Merged>("merge_timesheet_entries", { rows: rows.map(plainRow) }),
  unmergeTimesheetEntries: (id: string, removed: Merged["removed"]) =>
    invoke<void>("unmerge_timesheet_entries", { id, removed }),
  /** Takipte değişen satırları günceller; silinen (işi kalmayan) satır sayısını döndürür. */
  refreshTimesheetEntries: (ids: string[]) => invoke<number>("refresh_timesheet_entries", { ids }),
  /** Günün düzenlemelerini, elle eklenen ve gizlenen satırlarını siler; iş yeniden önerilir. */
  resetTimesheetDay: (timesheetId: string, date: string) =>
    invoke<number>("reset_timesheet_day", { timesheetId, date }),
  timesheetDetails: () => invoke<string[]>("timesheet_details"),
  pickTimesheetFile: () => invoke<string | null>("pick_timesheet_file"),
  /** `timesheetId` `null` ise yeni zaman çizelgesi. */
  importTimesheetTemplate: (timesheetId: string | null, path: string) =>
    invoke<Imported>("import_timesheet_template", { timesheetId, path }),
  exportTimesheet: (timesheetId: string, rows: RowRef[]) =>
    invoke<Exported>("export_timesheet", { timesheetId, rows: rows.map(plainRow) }),
  /** Son aktarımı geri alır (Sheets satırları silinir, Excel yedekten döner); sonuç iletisini verir. */
  undoLastExport: () => invoke<string>("undo_last_export"),
  sheetScript: () => invoke<string>("sheet_script"),
  /** Sheets API ile doğrudan bağlantı (Google hesabı). */
  googleStatus: () => invoke<GoogleStatus>("google_status"),
  /** Tarayıcıda Google girişi; `clientSecret` boşsa kayıtlı olan. */
  googleConnect: (clientId: string, clientSecret: string) =>
    invoke<GoogleStatus>("google_connect", { clientId, clientSecret }),
  googleCancel: () => invoke<void>("google_cancel"),
  googleDisconnect: () => invoke<GoogleStatus>("google_disconnect"),
  /** `timesheetId` `null` ise yeni zaman çizelgesi. */
  connectSheet: (timesheetId: string | null, url: string, link: string | null) =>
    invoke<Imported>("connect_sheet", { timesheetId, url, link }),
  disconnectSheet: (timesheetId: string) => invoke<TimesheetConfig>("disconnect_sheet", { timesheetId }),
  removeTimesheet: (timesheetId: string) => invoke<TimesheetConfig>("remove_timesheet", { timesheetId }),
  aiSettings: () => invoke<AiStatus>("get_ai_settings"),
  /** `apiKey` verilmezse kayıtlı anahtar korunur; boş dize siler. */
  saveAiSettings: (enabled: boolean, apiKey?: string) =>
    invoke<AiStatus>("save_ai_settings", { enabled, apiKey: apiKey ?? null }),
  testAi: (apiKey?: string) => invoke<string>("test_ai_connection", { apiKey: apiKey ?? null }),
  aiWriteDetails: (timesheetId: string, date: string, rewrite: boolean) =>
    invoke<AiChange[]>("ai_write_details", { timesheetId, date, rewrite }),
  /** Toplantı serisini projeye ata; `null` yoksayar. */
  assignMeeting: (uid: string, projectId: string | null) => invoke<void>("assign_meeting", { uid, projectId }),
  /** `start` gününden itibaren `days` günün takvim toplantıları; takvim bağlı değilse boş. */
  meetings: (start: string, days: number) => invoke<CalendarMeeting[]>("calendar_meetings", { start, days }),
  calendarStatus: () => invoke<CalendarStatus>("calendar_status"),
  setCalendarUrl: (url: string | null) => invoke<CalendarStatus>("set_calendar_url", { url }),
  refreshCalendar: () => invoke<void>("refresh_calendar"),
  restoreIgnoredMeetings: () => invoke<CalendarStatus>("restore_ignored_meetings"),
  onCalendar: (cb: (s: CalendarStatus) => void): Promise<UnlistenFn> =>
    listen<CalendarStatus>("calendar", (e) => cb(e.payload)),
  /** Aramayla eşleşen oturumları İndirilenler'e CSV yazar; dosya yolunu döndürür. */
  exportSearch: (query: string, start: string, days: number) => invoke<string>("export_search", { query, start, days }),
  acceptProject: (name: string) => invoke<Tag>("accept_project_suggestion", { name }),
  acceptCategory: (s: CategorySuggestion) =>
    invoke<void>("accept_category_suggestion", { field: s.field, pattern: s.pattern, categoryId: s.categoryId }),
  dismissSuggestion: (key: string) => invoke<void>("dismiss_suggestion", { key }),
  /** Aralıktaki oturumlara elle kategori; `null` kurallara döndürür. */
  setRangeCategory: (start: string, end: string, categoryId: string | null, scope: EditScope | null = null) =>
    invoke<Edited>("set_range_category", { start, end, categoryId, scope }),
  deleteRange: (start: string, end: string, scope: EditScope | null = null) =>
    invoke<Edited>("delete_range", { start, end, scope }),
  addManualEntry: (label: string, start: string, end: string, categoryId: string | null, projectId: string | null) =>
    invoke<Edited>("add_manual_entry", { label, start, end, categoryId, projectId }),
  /** Takvim bloğunu yeni aralığa uzatır ya da kısaltır (kısalan kısım silinir). */
  resizeBlock: (
    start: string,
    end: string,
    newStart: string,
    newEnd: string,
    label: string,
    categoryId: string | null,
    projectId: string | null,
  ) => invoke<Edited>("resize_block", { start, end, newStart, newEnd, label, categoryId, projectId }),
  /** Aralıktaki oturumlara elle proje; `null` kurallara döndürür. */
  /** Aralıkta kullanılan uygulamalar ("Son süreyi ata" önizlemesi). */
  rangeApps: (start: string, end: string) => invoke<UsageTotal[]>("range_apps", { start, end }),
  setRangeProject: (start: string, end: string, projectId: string | null, scope: EditScope | null = null) =>
    invoke<Edited>("set_range_project", { start, end, projectId, scope }),
  privacy: () => invoke<PrivacySettings>("get_privacy"),
  savePrivacy: (settings: PrivacySettings) => invoke<PrivacySettings>("save_privacy", { settings }),
  defaultExcludedUrls: () => invoke<string[]>("default_excluded_urls"),
  goals: () => invoke<Goals>("get_goals"),
  saveGoals: (goals: Goals) => invoke<void>("save_goals", { goals }),
  exportCsv: () => invoke<string>("export_csv"),
  backupStatus: () => invoke<BackupStatus>("backup_status"),
  backupNow: () => invoke<BackupFile>("backup_now"),
  openBackupFolder: () => invoke<void>("open_backup_folder"),
  /** Yedek dosyası seçtirir ve içeriğini döndürür; vazgeçilirse `null`. */
  pickBackup: () => invoke<PickedBackup | null>("pick_backup"),
  /** Yedeği geri yükler ve uygulamayı yeniden başlatır. */
  restoreBackup: (path: string) => invoke<void>("restore_backup", { path }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncConfigure: (url: string, anonKey: string, schema: string | null) =>
    invoke<SyncStatus>("sync_configure", { url, anonKey, schema }),
  syncSignIn: (email: string, password: string, signUp: boolean) =>
    invoke<SyncStatus>("sync_sign_in", { email, password, signUp }),
  syncSignOut: () => invoke<SyncStatus>("sync_sign_out"),
  syncDisconnect: () => invoke<SyncStatus>("sync_disconnect"),
  syncNow: () => invoke<void>("sync_now"),
  onSync: (cb: (s: SyncStatus) => void): Promise<UnlistenFn> => listen<SyncStatus>("sync", (e) => cb(e.payload)),
  diagnose: () => invoke<string>("diagnose"),
  /** Sürüm, izinler, takip/eşitleme durumu ve günlüklerin sonu (panoya kopyalanır). */
  diagnostics: () => invoke<string>("diagnostics"),
  revealLog: () => invoke<void>("reveal_log"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkUpdate: () => invoke<UpdateStatus>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
  onUpdate: (cb: (s: UpdateStatus) => void): Promise<UnlistenFn> =>
    listen<UpdateStatus>("update", (e) => cb(e.payload)),
  /** Menüden, menü çubuğundan ya da bildirimden gelen sayfa isteği ("day", "timesheet", "palette"…). */
  onNavigate: (cb: (target: string) => void): Promise<UnlistenFn> => listen<string>("navigate", (e) => cb(e.payload)),
  onStatus: (cb: (s: TrackingStatus) => void): Promise<UnlistenFn> =>
    listen<TrackingStatus>("status", (e) => cb(e.payload)),
};

/** Arka uca giden satır: görünümün alanları (kimlik, anahtar, durum) atılır. */
function plainEntry(e: TimesheetEntry): TimesheetEntry {
  return {
    date: e.date,
    start: e.start,
    hours: e.hours,
    actualHours: e.actualHours ?? null,
    kind: e.kind,
    details: e.details,
    party: e.party,
    projectId: e.projectId,
    division: e.division,
    coverage: e.coverage ?? null,
  };
}

function plainFileRow(r: FileRow): FileRow {
  return {
    row: r.row,
    date: r.date,
    start: r.start,
    hours: r.hours,
    kind: r.kind,
    details: r.details,
    party: r.party,
    division: r.division,
    consultant: r.consultant,
  };
}

function plainRow(r: RowRef): RowRef {
  return { id: r.id, entry: plainEntry(r.entry) };
}

/** "23dk", "1sa 5dk", "<1dk" — menü çubuğuyla aynı biçim. */
export function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return m === 0 ? (secs > 0 ? "<1dk" : "0dk") : `${m}dk`;
  return m === 0 ? `${h}sa` : `${h}sa ${m}dk`;
}
