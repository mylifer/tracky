import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronLeft,
  ChevronRight,
  CircleCheck,
  ClipboardCheck,
  Combine,
  Copy,
  FileSpreadsheet,
  FolderKanban,
  Loader2,
  RefreshCw,
  RotateCcw,
  Settings2,
  Sheet,
  Sparkles,
  Trash2,
  TriangleAlert,
  WandSparkles,
  X,
} from "lucide-react";
import {
  api,
  formatDuration,
  type CalendarStatus,
  type EntryKind,
  type EntryView,
  type Exported,
  type FileRow,
  type RowRef,
  type SheetRows,
  type SheetRowView,
  type Tag,
  type Timesheet as TimesheetInfo,
  type TimesheetConfig,
  type TimesheetDay,
  type TimesheetEntry,
  type UnassignedMeeting,
} from "../api";
import { useTauriEvent } from "../lib/useTauriEvent";
import { CalendarConnect, CONNECTIONS_SECTION, fileName, SheetConnect, TIMESHEET_SECTION } from "./TimesheetSettings";
import { ErrorText } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import {
  addDays,
  addMonths,
  daysInMonth,
  formatMonth,
  formatWeek,
  isoDate,
  parseIsoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "../lib/dates";
import { ProjectSelect } from "../components/ProjectSelect";
import { MonthBoard, WeekBoard } from "../components/TimesheetBoard";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { friendlyError, notifyChanged, toast, useChanged } from "../lib/feedback";
import {
  blocked,
  closeReport,
  copyDetails,
  UNASSIGNED_MIN,
  hoursDiff,
  isStale,
  mergeProblem,
  needsDetails,
  started,
  summarizeDay,
  type CloseReport,
} from "../lib/timesheet";

const KINDS: EntryKind[] = ["Working", "Online", "F2F"];
const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric", month: "short" });
const num = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });
/** Uyarı rozeti (saat tutmuyor gibi; aktarımı engellemez). */
const WARN_BADGE = "border-amber-500/40 text-amber-700 dark:text-amber-400";
/** Dönem denetimindeki düzeltme bağlantıları. */
const FIX_LINK = "rounded text-muted-foreground underline-offset-2 hover:text-foreground hover:underline";
/** Satır ızgarası: seçim, başlangıç, saat, tür, açıklama, taraf, birim, sil. */
const ROW_GRID = "grid-cols-[20px_104px_128px_96px_minmax(160px,1fr)_96px_minmax(110px,180px)_28px]";
/** Satır düzenlenirken kaydedilen alanlar (yeniden yüklemede yazılanın üzerine yazılmaz). */
const EDITABLE = ["start", "hours", "kind", "details", "party", "division"] as const;

/** Takvimde "yoksay" seçeneğinin değeri. */
const IGNORE = "__yoksay__";

function exportNotice(r: Exported) {
  const where = r.sheets ? `Google Sheets'e (${r.target})` : "Excel'e";
  const skipped = r.skipped ? `, ${r.skipped} satır zaten yazılmıştı` : "";
  const backup = r.backup ? ` Yedek: ${r.backup}` : "";
  return `${r.rows} satır ${where} eklendi (${r.filled} boş satıra, ${r.inserted} yeni satır${skipped}).${backup}`;
}

type Mode = "day" | "week" | "month";
const MODES: { id: Mode; label: string; current: string; close: string }[] = [
  { id: "day", label: "Gün", current: "Bugün", close: "Günü kapat" },
  { id: "week", label: "Hafta", current: "Bu hafta", close: "Haftayı kapat" },
  { id: "month", label: "Ay", current: "Bu ay", close: "Ayı kapat" },
];
const MODE_KEY = "kum.timesheet.mode";
/** Son açılan zaman çizelgesi (birden çok firma varsa). */
const SHEET_KEY = "kum.timesheet.sheet";
const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

function savedMode(): Mode {
  try {
    const m = localStorage.getItem(MODE_KEY);
    if (m === "day" || m === "week" || m === "month") return m;
  } catch {
    // Depolama kapalıysa varsayılan.
  }
  return "week";
}

function savedSheet(): string | null {
  try {
    return localStorage.getItem(SHEET_KEY);
  } catch {
    return null;
  }
}

/** Görünümün ilk günü ve gün sayısı; `anchor` görünümdeki herhangi bir gün. */
function range(mode: Mode, anchor: Date): { start: Date; days: number } {
  if (mode === "day") return { start: anchor, days: 1 };
  if (mode === "week") return { start: startOfWeek(anchor), days: 7 };
  const start = startOfMonth(anchor);
  return { start, days: daysInMonth(start) };
}

function rangeTitle(mode: Mode, start: Date) {
  if (mode === "day") return longDate.format(start);
  if (mode === "week") return formatWeek(start);
  const m = formatMonth(start);
  return m.charAt(0).toUpperCase() + m.slice(1);
}

/** Gerçek süre (saat) → "1sa 7dk". */
function actual(hours: number) {
  return formatDuration(Math.round(hours * 3600));
}

/** Kayıtların gerçek süresi; bilinmiyorsa yazılan saat. */
function worked(e: TimesheetEntry) {
  return e.actualHours ?? e.hours;
}

/** Saat, adam-gün (saat / günlük saat; yuvarlanmaz). */
function manDays(hours: number, dayHours: number) {
  return `${num.format(hours)} sa · ${num.format(hours / (dayHours || 8))} ag`;
}

/** Günlük saatten fark: "+0,50 sa", "−1,50 sa". */
function signedHours(diff: number) {
  return `${diff > 0 ? "+" : "−"}${num.format(Math.abs(diff))} sa`;
}

function toRef(e: EntryView): RowRef {
  return { id: e.id, entry: e };
}

/** Projenin adı; çizelgenin eşlemesindeki birim değil, Kum'daki proje. */
function projectName(projects: Tag[], id: string) {
  return projects.find((p) => p.id === id)?.name;
}

/** Projenin satırlarına yazılan birim: eşlemedeki birim, yoksa proje adı. */
function defaultDivision(sheet: TimesheetInfo, projects: Tag[], projectId: string) {
  const m = sheet.projects.find((x) => x.projectId === projectId);
  return m?.division.trim() || projectName(projects, projectId) || "";
}

/** Gün kartını ortaya getirir; `focusEmpty` ise ilk boş açıklamaya odaklanır. */
function showDay(date: string, focusEmpty = false) {
  const card = document.getElementById(`gun-${date}`);
  if (!card) return;
  card.scrollIntoView({ behavior: "smooth", block: "center" });
  card.animate([{ boxShadow: "0 0 0 2px var(--ring)" }, { boxShadow: "0 0 0 0 transparent" }], { duration: 1400 });
  if (focusEmpty)
    card.querySelector<HTMLInputElement>("input[data-empty]:not(:disabled)")?.focus({ preventScroll: true });
}

/** Açıklama kopyalarken geriye bakılan gün sayısı (hafta sonu ve izin günlerini aşsın). */
const COPY_LOOKBACK = 7;

type Saved = { before: EntryView; id: string };

/**
 * Kaydedilen satırları geri alır: önceden canlı olan satır silinir (yeniden takipten gelir),
 * kaydedilmiş satır önceki haline döner.
 */
function undoSaves(saved: Saved[]) {
  Promise.all(
    saved.map(({ before, id }) =>
      before.id ? api.saveTimesheetEntry(before.id, before) : api.deleteTimesheetEntry(id),
    ),
  ).then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" }));
}

/**
 * Satırlara açıklama yazar (canlı satır kaydedilir); yazılanlar için "Geri al"lı bildirim.
 * Yarıda hata olursa o ana kadar yazılanlar kalır ve yine geri alınabilir.
 */
async function writeDetails(changes: { entry: EntryView; details: string }[], message: (n: number) => string) {
  const saved: Saved[] = [];
  try {
    for (const c of changes)
      saved.push({ before: c.entry, id: await api.saveTimesheetEntry(c.entry.id, { ...c.entry, details: c.details }) });
  } finally {
    if (saved.length > 0)
      toast(message(saved.length), { tone: "success", action: { label: "Geri al", run: () => undoSaves(saved) } });
  }
}

/** Önceki günlerin aynı projedeki açıklamalarını günün boş açıklamalı satırlarına yazar ([copyDetails]). */
async function copyPreviousDetails(sheetId: string, date: string) {
  const [[day], prior] = await Promise.all([
    api.timesheetDays(sheetId, date, 1),
    api.timesheetDays(sheetId, isoDate(addDays(parseIsoDate(date), -COPY_LOOKBACK)), COPY_LOOKBACK),
  ]);
  prior.reverse();
  const changes = copyDetails(day.entries, prior);
  if (changes.length === 0) {
    toast("Önceki günlerde bu projelere yazılmış açıklama yok");
    return;
  }
  await writeDetails(changes, (n) => `${n} satıra önceki günlerden açıklama yazıldı`);
}

/**
 * Günlerin açıklamalarını yapay zekâyla (Claude) yazar: gün başına bir istek, sırayla. Varsayılan
 * olarak yalnızca boş ya da otomatik gelen (başlıklardan, hazır açıklamadan) açıklamalar yazılır,
 * elle yazılana dokunulmaz; `rewrite` ise günün aktarılmamış bütün satırları. Yazılanlar hemen
 * kaydedilir, bildirimden hepsi birden geri alınır. Bir gün hata verirse sonraki günlere
 * geçilmez; o ana kadar yazılanlar kalır.
 */
async function aiWriteDays(sheetId: string, dates: string[], rewrite: boolean, onProgress?: (done: number) => void) {
  const saved: Saved[] = [];
  let failure: unknown = null;
  for (const [i, date] of dates.entries()) {
    try {
      const [day] = await api.timesheetDays(sheetId, date, 1);
      if (!day.entries.some((e) => !e.exported)) continue;
      for (const w of await api.aiWriteDetails(sheetId, date, rewrite)) {
        const entry = day.entries.find((e) => e.key === w.key);
        if (!entry || entry.details === w.details) continue;
        saved.push({ before: entry, id: await api.saveTimesheetEntry(entry.id, { ...entry, details: w.details }) });
      }
    } catch (e) {
      failure = e;
      break;
    } finally {
      onProgress?.(i + 1);
    }
  }
  if (saved.length > 0)
    toast(`${saved.length} satıra yapay zekâyla açıklama yazıldı`, {
      tone: "success",
      action: { label: "Geri al", run: () => undoSaves(saved) },
    });
  else if (!failure) toast("Yazılacak açıklama yok: boş ya da otomatik açıklamalı, aktarılmamış satır kalmadı");
  if (failure) throw failure;
}

/** Satırları gizler (siler); bildirimden geri alınır. */
async function dismissRows(rows: EntryView[]) {
  const done: { row: EntryView; id: string }[] = [];
  try {
    for (const row of rows) done.push({ row, id: await api.dismissTimesheetEntry(row.id, row) });
  } finally {
    if (done.length > 0)
      toast(done.length === 1 ? "Satır silindi" : `${done.length} satır silindi`, {
        action: {
          label: "Geri al",
          run: () => {
            const saved = done.filter((d) => d.row.id).map((d) => d.id);
            // Canlı satır gizlenince kaydedilmişti: silinince yeniden takipten gelir.
            const live = done.filter((d) => !d.row.id).map((d) => d.id);
            Promise.all([
              saved.length ? api.undismissTimesheetEntries(saved) : null,
              ...live.map((id) => api.deleteTimesheetEntry(id)),
            ]).then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" }));
          },
        },
      });
  }
}

/** Satırları birleştirir; bildirimden geri alınır. */
async function mergeRows(rows: EntryView[]) {
  const merged = await api.mergeTimesheetEntries(rows.map(toRef));
  toast(`${rows.length} satır birleştirildi`, {
    tone: "success",
    action: {
      label: "Geri al",
      run: () =>
        api
          .unmergeTimesheetEntries(merged.id, merged.removed)
          .then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" })),
    },
  });
}

/** Takipte değişen satırları günceller (projede işi kalmayan satır silinir). */
async function refreshRows(ids: string[]) {
  const removed = await api.refreshTimesheetEntries(ids);
  const updated = ids.length - removed;
  toast(
    [updated ? `${updated} satır güncellendi` : "", removed ? `${removed} satırın projede işi kalmadı, silindi` : ""]
      .filter(Boolean)
      .join("; "),
    { tone: "success" },
  );
}

/**
 * Zaman çizelgesi: seçili firmanın çizelgesine bağlı projelere atanan süreden iş kayıtları.
 * Raporda projeye atanan süre burada hemen satır olur; düzenlenen satır kaydedilir, sonradan
 * atanan iş yeni satır olarak eklenir. Satırlar seçilip birleştirilir ya da gönderilir.
 */
export default function Timesheet({
  onOpenDay,
  onReviewDay,
  onOpenSettings,
}: {
  onOpenDay: (iso: string) => void;
  /** Günün projeye atanmamış süresini Gözden geçir'de aç. */
  onReviewDay: (iso: string) => void;
  /** Ayarlar'ı bu bölümle aç (Bağlantılar ya da Zaman çizelgeleri). */
  onOpenSettings: (section: string) => void;
}) {
  const [mode, setModeState] = useState<Mode>(savedMode);
  // Görünümdeki herhangi bir gün; görünüm değişince aynı gün etrafında açılır.
  const [anchor, setAnchor] = useState(() => isoDate(today()));
  const setMode = (m: Mode) => {
    setModeState(m);
    try {
      localStorage.setItem(MODE_KEY, m);
    } catch {
      // Hatırlanmasa da olur.
    }
  };
  const [sheetId, setSheetIdState] = useState<string | null>(savedSheet);
  const setSheetId = (id: string) => {
    setSheetIdState(id);
    try {
      localStorage.setItem(SHEET_KEY, id);
    } catch {
      // Hatırlanmasa da olur.
    }
  };
  const { start: rangeStart, days: rangeDays } = range(mode, parseIsoDate(anchor));
  const start = isoDate(rangeStart);
  const [config, setConfig] = useState<TimesheetConfig | null>(null);
  const sheet = config?.timesheets.find((t) => t.id === sheetId) ?? config?.timesheets[0];
  const [days, setDays] = useState<TimesheetDay[]>([]);
  // Ekrandaki günlerin çizelgesi ve aralığı: yeni günler yüklenene kadar gönderme eski satırlarla çalışmasın.
  const [daysOf, setDaysOf] = useState("");
  const fresh = !!sheet && daysOf === `${sheet.id}/${start}/${rangeDays}`;
  const [details, setDetails] = useState<string[]>([]);
  const [projects, setProjects] = useState<Tag[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** Bildirimdeki ileti son aktarımın sonucuysa geri alınabilir. */
  const [undoable, setUndoable] = useState(false);
  const [undoing, setUndoing] = useState(false);
  const exported = (r: Exported) => {
    setNotice(exportNotice(r));
    setUndoable(true);
  };
  // Gönderim sürerken düğmeler kilitli: çift tıklama aynı satırları dosyaya iki kez yazmasın.
  const [exporting, setExporting] = useState(false);
  const [calendar, setCalendar] = useState<CalendarStatus | null>(null);
  // Dönemi kapatma denetimi açık.
  const [closing, setClosing] = useState(false);
  // Yapay zekâyla yazma açık ve anahtar girilmiş (Ayarlar → Yapay zekâ).
  const [ai, setAi] = useState(false);
  // Hafta ve ay panosunda satırları açılan gün (dönemin dışındaysa varsayılan gün).
  const [picked, setPicked] = useState<string | null>(null);
  // Seçili satırlar (anahtarlarıyla): birleştir, gönder, sil.
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  useEffect(() => {
    api.aiSettings().then(
      (s) => setAi(s.enabled && s.hasKey),
      () => {},
    );
  }, []);

  // Çizelgenin dosyasındaki satırlar (gün ve hafta görünümünde): hangi çizelge ve aralığın
  // olduğu, okunuyor mu, okunamadıysa neden.
  const [file, setFile] = useState<{ of: string; data: SheetRows } | null>(null);
  const [fileError, setFileError] = useState<{ of: string; message: string } | null>(null);
  const [fileLoading, setFileLoading] = useState(false);

  // Hafta hızla değiştirilince geç gelen eski yanıt yenisinin üzerine yazmasın.
  const loadSeq = useRef(0);
  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    try {
      const c = await api.timesheetConfig();
      const t = c.timesheets.find((x) => x.id === sheetId) ?? c.timesheets[0];
      const [d, det, tax] = await Promise.all([
        t ? api.timesheetDays(t.id, start, rangeDays) : Promise.resolve<TimesheetDay[]>([]),
        api.timesheetDetails(),
        api.taxonomy(),
      ]);
      if (seq !== loadSeq.current) return;
      setConfig(c);
      setDays(d);
      setDaysOf(t ? `${t.id}/${start}/${rangeDays}` : "");
      setDetails(det);
      setProjects(tax.tags.filter((x) => x.kind === "project"));
      setError(null);
    } catch (e) {
      if (seq === loadSeq.current) setError(friendlyError(e));
    }
  }, [sheetId, start, rangeDays]);
  useEffect(() => {
    load();
    api.calendarStatus().then(setCalendar, () => {});
  }, [load]);
  useChanged(load);

  const fileSeq = useRef(0);
  const fileSheet = config?.timesheets.find((t) => t.id === sheetId) ?? config?.timesheets[0];
  const fileOf =
    fileSheet && (fileSheet.sheetUrl || fileSheet.filePath) ? `${fileSheet.id}/${start}/${rangeDays}` : null;
  /** Dosyanın satırlarını okur; dosyada değiştirilen satırlar Kum'a geçtiyse günler yeniden yüklenir. */
  const loadFile = useCallback(async () => {
    const seq = ++fileSeq.current;
    if (!fileOf || !fileSheet) return;
    setFileLoading(true);
    try {
      const data = await api.sheetRows(fileSheet.id, start, rangeDays);
      if (seq !== fileSeq.current) return;
      setFile({ of: fileOf, data });
      setFileError(null);
      if (data.synced > 0) await load();
    } catch (e) {
      if (seq === fileSeq.current) setFileError({ of: fileOf, message: friendlyError(e) });
    } finally {
      if (seq === fileSeq.current) setFileLoading(false);
    }
    // `fileSheet` her yüklemede yeni nesne: yalnızca kimliği ve dosyası önemli.
  }, [fileOf, fileSheet?.sheetUrl, fileSheet?.filePath, start, rangeDays, load]);
  useEffect(() => {
    loadFile();
  }, [loadFile]);
  // Takvim arka planda yenilenince toplantılar değişmiş olabilir.
  useTauriEvent(api.onCalendar, (s) => {
    setCalendar(s);
    load();
  });
  // Ekrandan kalkan (gönderilen, silinen, birleşen) satırlar seçimden çıkar.
  useEffect(() => {
    setSelected((s) => {
      const keys = new Set(days.flatMap((d) => d.entries.filter((e) => !e.exported).map((e) => e.key)));
      const next = new Set([...s].filter((k) => keys.has(k)));
      return next.size === s.size ? s : next;
    });
  }, [days]);

  // Klavye: ←/→ önceki/sonraki dönem, T bugün (rapor sayfalarındaki gibi), Esc seçimi kaldırır.
  // İşleyici her çizimde güncellenir; dinleyici bir kez eklenir.
  const keys = useRef<((e: KeyboardEvent) => void) | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current?.(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      await load();
    } catch (e) {
      setError(friendlyError(e));
    }
  };
  /** Dosyaya da yazan işlemler: sonra dosyanın satırları yeniden okunur (hata olsa da). */
  const runFile: Run = (f) => async () => {
    await run(f)();
    void loadFile();
  };

  /** Satırları çizelgenin dosyasına gönderir (canlı satırlar önce kaydedilir). */
  const send = async (rows: EntryView[], after?: () => void) => {
    if (!sheet || rows.length === 0) return;
    setExporting(true);
    setError(null);
    try {
      exported(await api.exportTimesheet(sheet.id, rows.map(toRef)));
      setSelected(new Set());
      after?.();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setExporting(false);
      await load();
      void loadFile();
    }
  };

  if (!config)
    return error ? (
      <ErrorText>{error}</ErrorText>
    ) : (
      <div className="mx-auto w-full max-w-5xl space-y-3 px-6 pt-2 pb-10" aria-busy>
        <div className="skeleton h-12 w-72 rounded-lg" />
        {[0, 1, 2].map((i) => (
          <div key={i} className="skeleton h-28 rounded-xl" style={{ animationDelay: `${i * 120}ms` }} />
        ))}
      </div>
    );
  if (!sheet) return <Setup onDone={load} />;
  const target = sheet.sheetUrl
    ? "Google Sheets"
    : sheet.filePath
      ? `Excel · ${fileName(sheet.filePath)}`
      : "Dosya seçilmedi";
  const sendLabel = sheet.sheetUrl ? "Sheets'e gönder" : "Excel'e aktar";
  const calendarText = !calendar?.url
    ? "Outlook takvimi bağlı değil"
    : calendar.last && !calendar.last.ok
      ? "Outlook takvimi okunamadı"
      : "Outlook takvimi bağlı";
  const sheetProjects = sheet.projects
    .map((m) => projects.find((p) => p.id === m.projectId))
    .filter((p): p is Tag => !!p);

  const all = days.flatMap((d) => d.entries);
  // Dosyanın bu döneme ait satırları (eski aralığın yanıtı gösterilmez).
  const fileData = file && file.of === fileOf ? file.data : null;
  const fileRowOf = new Map<string, number>();
  for (const r of fileData?.rows ?? []) if (r.entryId) fileRowOf.set(r.entryId, r.row);
  const missing = new Set(fileData?.missing ?? []);
  const outside = (fileData?.rows ?? []).filter((r) => !r.entryId);
  const outsideHours = new Map<string, number>();
  for (const r of outside) outsideHours.set(r.date, (outsideHours.get(r.date) ?? 0) + (r.hours ?? 0));
  const unsent = all.filter((e) => !e.exported);
  // Toplu gönderim yalnızca başlamış işi gönderir; ilerideki satırlar seçilerek gönderilebilir.
  const now = new Date();
  const pending = unsent.filter((e) => started(e, now));
  const later = unsent.length - pending.length;
  const selectedRows = unsent.filter((e) => selected.has(e.key));
  const report = closeReport(days, config.dayHours, isoDate(today()), outsideHours);
  const total = all.reduce((s, e) => s + e.hours, 0) + outside.reduce((s, r) => s + (r.hours ?? 0), 0);
  const totalActual = all.reduce((s, e) => s + worked(e), 0);
  const byDivision = new Map<string, number>();
  for (const e of all) byDivision.set(e.division, (byDivision.get(e.division) ?? 0) + e.hours);
  for (const r of outside) byDivision.set(r.division, (byDivision.get(r.division) ?? 0) + (r.hours ?? 0));
  const fileState: FileState = !fileOf
    ? { kind: "off" }
    : fileData
      ? { kind: "ready", rowOf: fileRowOf, missing }
      : fileError?.of === fileOf
        ? { kind: "error" }
        : { kind: "loading" };
  const current = isoDate(range(mode, today()).start);
  const step = (n: number) => {
    const a = parseIsoDate(start);
    setAnchor(isoDate(mode === "day" ? addDays(a, n) : mode === "week" ? addDays(a, 7 * n) : addMonths(a, n)));
  };
  const modeInfo = MODES.find((m) => m.id === mode)!;
  keys.current = (e) => {
    if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]")) return;
    if (e.key === "ArrowLeft") step(-1);
    else if (e.key === "ArrowRight" && start < current) step(1);
    else if (e.key === "t" || e.key === "T") setAnchor(isoDate(today()));
    else if (e.key === "Escape" && selected.size > 0) setSelected(new Set());
    else return;
    e.preventDefault();
  };
  // Panoda açılan gün: seçilen, yoksa bugün (dönemdeyse), yoksa satırı olan ilk gün.
  const todayIso = isoDate(today());
  const shown =
    days.find((d) => d.date === picked)?.date ??
    days.find((d) => d.date === todayIso)?.date ??
    days.find((d) => d.entries.length > 0 || outsideHours.has(d.date))?.date ??
    days[0]?.date;
  const shownDay = days.find((d) => d.date === shown);
  /** Günün kartını açıp gösterir (dönem denetiminin "göster" bağlantısı). */
  const showDayCard = (iso: string, focusEmpty = false) => {
    setPicked(iso);
    setTimeout(() => showDay(iso, focusEmpty), 60);
  };
  // Birimlerin renk sırası: çizelgenin projelerinin birimleri, dosyadaki birimler, sonra satırlarda görülenler.
  const divisionList: string[] = [];
  const addDivision = (d: string) => {
    const t = d.trim();
    if (!divisionList.some((x) => x.toLocaleLowerCase("tr") === t.toLocaleLowerCase("tr"))) divisionList.push(t);
  };
  for (const m of sheet.projects) addDivision(defaultDivision(sheet, projects, m.projectId));
  for (const d of sheet.divisions) addDivision(d);
  for (const e of all) addDivision(e.division);
  for (const r of outside) addDivision(r.division);
  const summaries = days.map((d) =>
    summarizeDay(
      d,
      outside.filter((r) => r.date === d.date),
      config.dayHours,
      todayIso,
    ),
  );
  const dayCard = (d: TimesheetDay, alwaysShow: boolean) => (
    <DayCard
      key={d.date}
      day={d}
      config={config}
      sheet={sheet}
      projects={projects}
      onOpenDay={onOpenDay}
      onReviewDay={onReviewDay}
      run={run}
      runFile={runFile}
      file={fileState}
      outside={outside.filter((r) => r.date === d.date)}
      ai={ai}
      selected={selected}
      onToggle={toggle}
      alwaysShow={alwaysShow}
    />
  );
  const toggle = (rowKeys: string[], on: boolean) =>
    setSelected((s) => {
      const next = new Set(s);
      for (const k of rowKeys) {
        if (on) next.add(k);
        else next.delete(k);
      }
      return next;
    });
  const sendBlocked = pending.some(blocked);

  return (
    <div className="mx-auto w-full max-w-5xl space-y-4 px-6 pt-2 pb-10">
      {config.timesheets.length > 1 && (
        <Tabs value={sheet.id} onValueChange={(v) => setSheetId(v)}>
          <TabsList aria-label="Zaman çizelgesi">
            {config.timesheets.map((t) => (
              <TabsTrigger key={t.id} value={t.id} className="px-3">
                {t.company || "Adsız çizelge"}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <div className="mr-auto">
          <h1 className="text-[15px] font-semibold">
            {sheet.company || "Zaman çizelgesi"} · {rangeTitle(mode, rangeStart)}
          </h1>
          <p className="text-xs text-muted-foreground">
            Toplam {manDays(total, config.dayHours)}
            {all.length > 0 && <span title="Takip edilen gerçek süre"> (gerçek {actual(totalActual)})</span>}
            {[...byDivision.entries()].map(([d, h]) => ` · ${d}: ${num.format(h)} sa`).join("")}
          </p>
          {/* Kayıtların nereden gelip nereye gittiği; tıklayınca Ayarlar. */}
          <div className="mt-0.5 flex flex-wrap items-center gap-x-1.5 text-[11px] text-muted-foreground">
            <button
              className="flex items-center gap-1 underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onOpenSettings(TIMESHEET_SECTION)}
              title="Bu çizelgeye yalnızca bu projelerin işi gider (Ayarlar → Zaman çizelgeleri)"
            >
              <FolderKanban className="size-3" />
              {sheetProjects.length ? (
                sheetProjects.map((p) => (
                  <span key={p.id} className="flex items-center gap-1">
                    <i className="size-1.5 rounded-full" style={{ background: tagColor(p) }} />
                    {p.name}
                  </span>
                ))
              ) : (
                <span className="text-amber-700 dark:text-amber-400">proje seçilmedi</span>
              )}
            </button>
            <span aria-hidden>·</span>
            <button
              className={cn(
                "flex items-center gap-1 underline-offset-2 hover:text-foreground hover:underline",
                !sheet.filePath && !sheet.sheetUrl && "text-amber-700 dark:text-amber-400",
              )}
              onClick={() => onOpenSettings(TIMESHEET_SECTION)}
              title="Ayarlar → Zaman çizelgeleri"
            >
              {sheet.sheetUrl ? <Sheet className="size-3" /> : <FileSpreadsheet className="size-3" />}
              <span className="max-w-60 truncate">{target}</span>
            </button>
            <span aria-hidden>·</span>
            <button
              className="flex items-center gap-1 underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onOpenSettings(CONNECTIONS_SECTION)}
              title="Ayarlar → Bağlantılar"
            >
              <CalendarDays className="size-3" />
              <span className={cn(calendar?.last && !calendar.last.ok && "text-destructive")}>{calendarText}</span>
            </button>
          </div>
        </div>
        <Tabs value={mode} onValueChange={(v) => setMode(v as Mode)}>
          <TabsList aria-label="Görünüm">
            {MODES.map((m) => (
              <TabsTrigger key={m.id} value={m.id} className="px-3">
                {m.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Önceki ${modeInfo.label.toLowerCase()}`}
          onClick={() => step(-1)}
        >
          <ChevronLeft />
        </Button>
        <Button variant="outline" size="sm" disabled={start === current} onClick={() => setAnchor(isoDate(today()))}>
          {modeInfo.current}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Sonraki ${modeInfo.label.toLowerCase()}`}
          disabled={start >= current}
          onClick={() => step(1)}
        >
          <ChevronRight />
        </Button>
        <Button variant="outline" size="sm" onClick={() => onOpenSettings(TIMESHEET_SECTION)}>
          <Settings2 /> Ayarlar
        </Button>
        <Button
          variant={closing ? "secondary" : "outline"}
          size="sm"
          aria-expanded={closing}
          onClick={() => setClosing((v) => !v)}
          title="Göndermeden önce dönemi denetle: atanmamış süre, toplantılar, saatler, açıklamalar"
        >
          <ClipboardCheck /> {modeInfo.close}
          {fresh && report.issues > 0 && (
            <span className="rounded-full bg-amber-500/15 px-1.5 text-[11px] text-amber-700 tabular dark:text-amber-400">
              {report.issues}
            </span>
          )}
        </Button>
        <Button
          size="sm"
          disabled={!fresh || pending.length === 0 || exporting || sendBlocked}
          title={
            pending.length === 0
              ? "Gönderilecek satır yok"
              : sendBlocked
                ? "Açıklaması boş ya da takipte değişen satırlar var: düzelt ya da gönderilecekleri seç"
                : `Bu dönemin gönderilmemiş ${pending.length} satırı${later ? ` (henüz başlamamış ${later} satır hariç)` : ""}: ${sheet.sheetUrl ? (sheet.sheetLink ?? "Google Sheets") : (sheet.filePath ?? "")}`
          }
          onClick={() => send(pending)}
        >
          {exporting ? <Loader2 className="animate-spin" /> : sheet.sheetUrl ? <Sheet /> : <FileSpreadsheet />}
          {sendLabel}
          {pending.length ? ` (${pending.length})` : ""}
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
      {fileOf && (
        <FileNotice
          sheets={!!sheet.sheetUrl}
          rows={fileData?.rows.length ?? null}
          outside={outside.length}
          missing={missing.size}
          loading={fileLoading}
          error={fileError?.of === fileOf ? fileError.message : null}
          onReload={() => void loadFile()}
        />
      )}
      {notice && (
        <div className="flex items-start gap-2 rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-xs">
          <Check className="mt-0.5 size-3.5 shrink-0 text-success" />
          <span className="min-w-0 flex-1 break-words selectable">{notice}</span>
          {undoable && (
            <button
              disabled={undoing}
              className="-my-0.5 shrink-0 rounded px-1.5 py-0.5 font-medium underline-offset-2 hover:bg-accent hover:underline disabled:opacity-50"
              onClick={async () => {
                setUndoing(true);
                setError(null);
                try {
                  setNotice(await api.undoLastExport());
                  setUndoable(false);
                } catch (e) {
                  setError(friendlyError(e));
                } finally {
                  setUndoing(false);
                  await load();
                  void loadFile();
                }
              }}
            >
              {undoing ? "Geri alınıyor…" : "Aktarımı geri al"}
            </button>
          )}
          <button
            aria-label="Kapat"
            className="-m-1 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
            onClick={() => setNotice(null)}
          >
            <X className="size-3.5" />
          </button>
        </div>
      )}
      {sheet.projects.length === 0 && (
        <ProjectsPrompt config={config} sheet={sheet} projects={projects} onSaved={load} onError={setError} />
      )}
      {closing && (
        <ClosePanel
          title={modeInfo.close}
          report={report}
          days={days}
          config={config}
          sheet={sheet}
          projects={projects}
          ready={fresh && !exporting}
          ai={ai}
          pending={pending}
          onClose={() => setClosing(false)}
          onSend={() => send(pending, () => setClosing(false))}
          onOpenDay={onOpenDay}
          onReviewDay={onReviewDay}
          onShowDay={showDayCard}
          run={run}
        />
      )}

      <datalist id="timesheet-details">
        {details.slice(0, 200).map((d) => (
          <option key={d} value={d} />
        ))}
      </datalist>
      <datalist id="timesheet-parties">
        {[...new Set([sheet.defaultParty, sheet.company, ...sheet.projects.map((p) => p.party ?? "")])]
          .filter(Boolean)
          .map((p) => (
            <option key={p} value={p} />
          ))}
      </datalist>

      {mode === "day" ? (
        // Gün görünümünde boş gün de gösterilir (yoksa sayfa boş kalır).
        days.map((d) => dayCard(d, true))
      ) : (
        <>
          {mode === "week" ? (
            <WeekBoard
              summaries={summaries}
              divisions={divisionList}
              dayHours={config.dayHours}
              selected={shown ?? null}
              onSelect={setPicked}
              todayIso={todayIso}
            />
          ) : (
            <MonthBoard
              summaries={summaries}
              divisions={divisionList}
              dayHours={config.dayHours}
              selected={shown ?? null}
              onSelect={setPicked}
              todayIso={todayIso}
            />
          )}
          {shownDay && dayCard(shownDay, true)}
        </>
      )}
      {selectedRows.length > 0 && (
        <SelectionBar
          rows={selectedRows}
          sendLabel={sendLabel}
          busy={exporting}
          onClear={() => setSelected(new Set())}
          onMerge={run(() => mergeRows(selectedRows).then(() => setSelected(new Set())))}
          onSend={() => send(selectedRows)}
          onDismiss={run(() => dismissRows(selectedRows).then(() => setSelected(new Set())))}
        />
      )}
    </div>
  );
}

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

/**
 * Dosyanın satırlarının durumu: okunduysa Kum'un aktardığı kayıtların dosyadaki satır numarası
 * ve dosyada bulunamayan kayıtlar. Aktarılmış satır yalnızca dosyadaki satırı bilinince düzenlenir.
 */
type FileState =
  { kind: "off" | "loading" | "error" } | { kind: "ready"; rowOf: Map<string, number>; missing: Set<string> };

/** Dosyanın adı, ekleriyle: "tablodan", "Excel dosyasından"… */
function fileWords(sheets: boolean) {
  return sheets
    ? { from: "tablodan", to: "tabloya", of: "tablonun", Of: "Tablonun", in: "tabloda", inAdj: "tablodaki" }
    : {
        from: "Excel dosyasından",
        to: "Excel dosyasına",
        of: "Excel dosyasının",
        Of: "Excel dosyasının",
        in: "Excel dosyasında",
        inAdj: "Excel dosyasındaki",
      };
}

/** Betik eski: yeni işlemleri (satırları okuma, değiştirme) tanımıyor. */
const OUTDATED = "betik eski";

/**
 * Dosyadaki satırların durumu: kaç satır okundu, kaçı Kum dışında girilmiş, okunamadıysa neden.
 * Betik eskiyse yeni betik kopyalanıp dağıtım güncellenir (adres değişmez).
 */
function FileNotice({
  sheets,
  rows,
  outside,
  missing,
  loading,
  error,
  onReload,
}: {
  sheets: boolean;
  rows: number | null;
  outside: number;
  missing: number;
  loading: boolean;
  error: string | null;
  onReload: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const where = sheets ? "Tablodaki" : "Excel dosyasındaki";
  const outdated = !!error?.includes(OUTDATED);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(await api.sheetScript());
      setCopied(true);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  };
  const reload = (
    <button
      className={cn(FIX_LINK, "flex items-center gap-1")}
      disabled={loading}
      onClick={onReload}
      title={`${where} satırları yeniden oku`}
    >
      {loading ? <Loader2 className="size-3 animate-spin" /> : <RefreshCw className="size-3" />}
      {loading ? "okunuyor…" : "yenile"}
    </button>
  );
  if (error)
    return (
      <div className="space-y-1.5 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <TriangleAlert className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
          <span className="min-w-0 flex-1 selectable">
            {outdated
              ? "Tablodaki betik eski: satırları buradan okuyup değiştirmek için yeni betik gerekiyor."
              : `${where} satırlar okunamadı: ${error}`}
          </span>
          {reload}
        </div>
        {outdated && (
          <div className="flex flex-wrap items-center gap-2 pl-5.5 text-muted-foreground">
            <Button size="sm" variant="outline" className="h-6 text-[11px]" onClick={copy}>
              {copied ? <Check /> : <Copy />} {copied ? "Kopyalandı" : "Yeni betiği kopyala"}
            </Button>
            <span>
              Tabloda <b>Uzantılar → Apps Script</b>'te içindekini silip yapıştır, kaydet; sonra{" "}
              <b>Dağıt → Dağıtımları yönet → ✎ → Sürüm: Yeni sürüm → Dağıt</b>. Adres değişmez.
            </span>
          </div>
        )}
      </div>
    );
  return (
    <div className="flex flex-wrap items-center gap-x-2 px-1 text-[11px] text-muted-foreground">
      {sheets ? <Sheet className="size-3" /> : <FileSpreadsheet className="size-3" />}
      {rows === null ? (
        <span>{where} satırlar okunuyor…</span>
      ) : (
        <span>
          {where} {rows} satır
          {outside > 0 && ` · ${outside} satır Kum dışında girilmiş`}
          {missing > 0 && (
            <span className="text-amber-700 dark:text-amber-400">
              {` · gönderilen ${missing} satır dosyada bulunamadı`}
            </span>
          )}
        </span>
      )}
      {rows !== null && reload}
    </div>
  );
}

/** Seçili satırların işlemleri: birleştir, gönder, sil. */
function SelectionBar({
  rows,
  sendLabel,
  busy,
  onClear,
  onMerge,
  onSend,
  onDismiss,
}: {
  rows: EntryView[];
  sendLabel: string;
  busy: boolean;
  onClear: () => void;
  onMerge: () => void;
  onSend: () => void;
  onDismiss: () => void;
}) {
  const hours = rows.reduce((s, e) => s + e.hours, 0);
  const mergeWhy = mergeProblem(rows);
  const blockedRows = rows.filter(blocked).length;
  return (
    <div className="sticky bottom-3 z-20 flex justify-center">
      <div
        role="toolbar"
        aria-label="Seçili satırlar"
        className="flex flex-wrap items-center gap-2 rounded-xl border bg-popover px-3 py-2 text-xs shadow-lg"
      >
        <span className="tabular">
          <b className="font-semibold">{rows.length}</b> satır seçildi · {num.format(hours)} sa
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={!!mergeWhy}
          title={mergeWhy ?? "En erken başlangıçta tek satır olur; süreler toplanır, açıklamalar birleşir"}
          onClick={onMerge}
        >
          <Combine /> Birleştir
        </Button>
        <Button
          size="sm"
          disabled={busy || blockedRows > 0}
          title={
            blockedRows > 0
              ? "Seçimde açıklaması boş ya da takipte değişen satır var"
              : "Yalnızca seçili satırları gönder"
          }
          onClick={onSend}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Sheet />} {sendLabel}
        </Button>
        <Button size="sm" variant="ghost" onClick={onDismiss} title="Satırları sil; bildirimden geri alınır">
          <Trash2 /> Sil
        </Button>
        <button
          aria-label="Seçimi kaldır"
          title="Seçimi kaldır (Esc)"
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={onClear}
        >
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

/** Çizelgede proje yoksa: hangi projelerin işi bu firmaya gidecek? */
function ProjectsPrompt({
  config,
  sheet,
  projects,
  onSaved,
  onError,
}: {
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  onSaved: () => void;
  onError: (e: string) => void;
}) {
  const taken = new Set(config.timesheets.flatMap((t) => t.projects.map((m) => m.projectId)));
  const free = projects.filter((p) => !taken.has(p.id));
  const add = async (projectId: string) => {
    try {
      await api.saveTimesheetConfig({
        ...config,
        timesheets: config.timesheets.map((t) =>
          t.id === sheet.id
            ? { ...t, projects: [...t.projects, { projectId, division: "", party: null, defaultDetails: null }] }
            : t,
        ),
      });
      onSaved();
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  return (
    <section className="space-y-2 rounded-xl border border-dashed bg-card px-4 py-3 shadow-xs">
      <div className="text-[13px] font-semibold">Bu zaman çizelgesine hangi projeler gitsin?</div>
      <p className="text-xs text-muted-foreground">
        Yalnızca seçtiğin projelere atanan süre {sheet.company ? `${sheet.company} çizelgesinde` : "burada"} satır olur
        ve tablosuna gönderilir. Bir proje tek bir çizelgeye bağlanır.
      </p>
      {free.length > 0 ? (
        <ProjectSelect
          value=""
          projects={free}
          placeholder="Proje ekle…"
          className="w-60"
          aria-label="Çizelgeye proje ekle"
          onChange={add}
        />
      ) : (
        <p className="text-xs text-muted-foreground">Boşta proje yok; kenar çubuğundaki Projeler'den ekle.</p>
      )}
    </section>
  );
}

/**
 * Dönemi kapatmadan önce denetim: atanmamış süre, projesi belli olmayan toplantılar, günlük
 * saati tutmayan ya da kaydı olmayan iş günleri, açıklaması boş ve takipte değişen satırlar;
 * her birinin yanında düzeltme bağlantısı. Boş açıklama ve takipte değişen satır gönderimi
 * engeller, diğerleri uyarıdır. Ana düğme dönemin gönderilmemiş satırlarını gönderir.
 */
function ClosePanel({
  title,
  report,
  days,
  config,
  sheet,
  projects,
  ready,
  ai,
  pending,
  onClose,
  onSend,
  onOpenDay,
  onReviewDay,
  onShowDay,
  run,
}: {
  title: string;
  report: CloseReport;
  days: TimesheetDay[];
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  /** Ekrandaki günler güncel ve gönderim sürmüyor. */
  ready: boolean;
  /** Yapay zekâyla yazma açık. */
  ai: boolean;
  /** Dönemin gönderilmemiş satırları. */
  pending: EntryView[];
  onClose: () => void;
  onSend: () => void;
  onOpenDay: (iso: string) => void;
  onReviewDay: (iso: string) => void;
  /** Günün kartını açıp gösterir; `focusEmpty` ise ilk boş açıklamaya odaklanır. */
  onShowDay: (iso: string, focusEmpty?: boolean) => void;
  run: Run;
}) {
  const label = (iso: string) => <span className="w-24 shrink-0 capitalize">{dayFmt.format(parseIsoDate(iso))}</span>;
  const show = (iso: string, focusEmpty = false) => (
    <button className={FIX_LINK} onClick={() => onShowDay(iso, focusEmpty)}>
      göster
    </button>
  );
  const calendarLink = (iso: string) => (
    <button className={FIX_LINK} onClick={() => onOpenDay(iso)} title="Takvimde aç: blokları ya da aralıkları ata">
      takvim
    </button>
  );
  const sendLabel = sheet.sheetUrl ? "Sheets'e gönder" : "Excel'e aktar";
  const isBlocked = report.blocking > 0;
  // Yapay zekâyla yazılacak günler: bugüne kadar, aktarılmamış satırı olanlar. Yazarken ilerleme.
  const todayIso = isoDate(today());
  const aiDays = days.filter((d) => d.date <= todayIso && d.entries.some((e) => !e.exported)).map((d) => d.date);
  const [aiDone, setAiDone] = useState<number | null>(null);
  const writeAll = async () => {
    setAiDone(0);
    try {
      await run(() => aiWriteDays(sheet.id, aiDays, false, setAiDone))();
    } finally {
      setAiDone(null);
    }
  };
  const staleIds = report.stale.flatMap((s) => s.rows.map((e) => e.id).filter((id): id is string => !!id));

  return (
    <section className="rounded-xl border bg-card shadow-xs" aria-label={`${title}: denetim`}>
      <div className="flex items-center gap-2 border-b px-4 py-2.5">
        <ClipboardCheck className="size-4 text-muted-foreground" />
        <span className="text-[13px] font-semibold">{title}</span>
        <span className="text-xs text-muted-foreground">
          {report.issues === 0 ? "göndermeden önce denetim" : `${report.issues} konu`}
        </span>
        <button
          aria-label="Kapat"
          className="-m-1 ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={onClose}
        >
          <X className="size-3.5" />
        </button>
      </div>
      {report.issues === 0 ? (
        <div className="m-3 flex items-start gap-2 rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-xs">
          <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-success" />
          <span>
            <b className="text-success">Hazır.</b> Açıklamalar dolu, iş günleri {num.format(config.dayHours)} saat;
            atanmamış süre ya da toplantı yok.
            {pending.length === 0 && " Gönderilecek satır kalmadı."}
          </span>
        </div>
      ) : (
        <ul className="divide-y">
          {report.stale.length > 0 && (
            <CheckGroup
              blocking
              title="Takipte değişen satırlar"
              hint="İşi raporda başka projeye alınmış; güncellenmeden gönderilmez."
            >
              {report.stale.map((s) => (
                <li key={s.date} className="flex flex-wrap items-center gap-2">
                  {label(s.date)}
                  <span>{s.rows.length} satır</span>
                  <span className="ml-auto flex gap-2">
                    <button
                      className={FIX_LINK}
                      onClick={run(() => refreshRows(s.rows.map((e) => e.id).filter((id): id is string => !!id)))}
                    >
                      güncelle
                    </button>
                    {show(s.date)}
                  </span>
                </li>
              ))}
              {report.stale.length > 1 && (
                <li className="flex justify-end">
                  <button className={cn(FIX_LINK, "text-[11px]")} onClick={run(() => refreshRows(staleIds))}>
                    Hepsini güncelle ({staleIds.length})
                  </button>
                </li>
              )}
            </CheckGroup>
          )}
          {report.details.length > 0 && (
            <CheckGroup blocking title="Açıklaması boş satırlar" hint="Doldurulmadan gönderilmez.">
              {report.details.map((d) => (
                <li key={d.date} className="flex flex-wrap items-center gap-2">
                  {label(d.date)}
                  <span>{d.rows} satır</span>
                  <span className="ml-auto flex gap-2">
                    <button className={FIX_LINK} onClick={run(() => copyPreviousDetails(sheet.id, d.date))}>
                      önceki günden kopyala
                    </button>
                    {ai && (
                      <button
                        className={FIX_LINK}
                        disabled={aiDone !== null}
                        onClick={run(() => aiWriteDays(sheet.id, [d.date], false))}
                      >
                        yapay zekâyla yaz
                      </button>
                    )}
                    {show(d.date, true)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.unassigned.length > 0 && (
            <CheckGroup title="Atanmamış süre" hint="Projeye atanmayan iş zaman çizelgesine girmez.">
              {report.unassigned.map((u) => (
                <li key={u.date} className="flex flex-wrap items-center gap-2">
                  {label(u.date)}
                  <span className="tabular">{formatDuration(u.seconds)}</span>
                  <span className="ml-auto flex gap-2">
                    <button className={FIX_LINK} onClick={() => onReviewDay(u.date)}>
                      gözden geçir
                    </button>
                    {calendarLink(u.date)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.meetings.length > 0 && (
            <CheckGroup
              title="Projesi belli olmayan toplantılar"
              hint="Seçilen proje serinin tüm tekrarlarına uygulanır."
            >
              {report.meetings.map((iso) => (
                <li key={iso} className="flex items-start gap-2">
                  <span className="pt-1">{label(iso)}</span>
                  <div className="min-w-0 flex-1">
                    <MeetingRows
                      meetings={days.find((d) => d.date === iso)?.meetings ?? []}
                      projects={projects}
                      run={run}
                    />
                  </div>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.hours.length > 0 && (
            <CheckGroup
              title="Saati tutmayan iş günleri"
              hint={`Günlük ${num.format(config.dayHours)} saat bekleniyor.`}
            >
              {report.hours.map((h) => (
                <li key={h.date} className="flex flex-wrap items-center gap-2">
                  {label(h.date)}
                  <span className="tabular">{num.format(h.hours)} sa</span>
                  <Badge variant="outline" className={WARN_BADGE}>
                    {signedHours(h.diff)}
                  </Badge>
                  <span className="ml-auto flex gap-2">
                    {calendarLink(h.date)}
                    {show(h.date)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.empty.length > 0 && (
            <CheckGroup
              title="Kaydı olmayan iş günleri"
              hint="İzin ya da tatilse geçebilirsin; değilse elle satır ekle."
            >
              {report.empty.map((iso) => (
                <li key={iso} className="flex flex-wrap items-center gap-2">
                  {label(iso)}
                  <span className="ml-auto flex gap-2">
                    {calendarLink(iso)}
                    {show(iso)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2 border-t px-4 py-2.5">
        <p className="mr-auto text-[11px] text-muted-foreground">
          {isBlocked
            ? "Açıklaması boş ya da takipte değişen satırlar düzeltilmeden gönderilmez."
            : report.issues > 0
              ? "Uyarılar göndermeyi engellemez."
              : ""}
        </p>
        {ai && aiDays.length > 0 && (
          <Button
            size="sm"
            variant="outline"
            disabled={!ready || aiDone !== null}
            title="Boş ya da otomatik gelen açıklamaları Claude yazar (gün başına bir istek); elle yazdıklarına dokunulmaz."
            onClick={writeAll}
          >
            {aiDone !== null ? <Loader2 className="animate-spin" /> : <Sparkles />}
            {aiDone !== null ? `Yazılıyor… ${aiDone}/${aiDays.length} gün` : "Boş açıklamaları yapay zekâyla yaz"}
          </Button>
        )}
        <Button
          size="sm"
          disabled={!ready || isBlocked || pending.length === 0}
          title={
            isBlocked
              ? "Önce açıklaması boş ve takipte değişen satırları düzelt"
              : pending.length === 0
                ? "Gönderilecek satır yok"
                : `Dönemin gönderilmemiş ${pending.length} satırı`
          }
          onClick={onSend}
        >
          {sheet.sheetUrl ? <Sheet /> : <FileSpreadsheet />}
          {sendLabel}
          {pending.length ? ` (${pending.length})` : ""}
        </Button>
      </div>
    </section>
  );
}

/** Denetimde bir konu ve etkilenen günler. */
function CheckGroup({
  title,
  hint,
  blocking,
  children,
}: {
  title: string;
  hint: string;
  blocking?: boolean;
  children: React.ReactNode;
}) {
  return (
    <li className="px-4 py-2.5">
      <div className="flex items-center gap-1.5 pb-1.5 text-xs">
        <TriangleAlert
          className={cn("size-3.5", blocking ? "text-destructive" : "text-amber-600 dark:text-amber-400")}
        />
        <span className="font-medium">{title}</span>
        <span className="text-muted-foreground">· {hint}</span>
      </div>
      <ul className="space-y-1 pl-5 text-xs">{children}</ul>
    </li>
  );
}

function DayCard({
  day,
  config,
  sheet,
  projects,
  onOpenDay,
  onReviewDay,
  run,
  runFile,
  file,
  outside,
  ai,
  selected,
  onToggle,
  alwaysShow,
}: {
  day: TimesheetDay;
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  onOpenDay: (iso: string) => void;
  onReviewDay: (iso: string) => void;
  run: Run;
  /** Dosyaya da yazan işlemler (sonra dosyanın satırları yeniden okunur). */
  runFile: Run;
  file: FileState;
  /** Günün dosyada Kum dışında girilmiş satırları. */
  outside: SheetRowView[];
  /** Yapay zekâyla yazma açık. */
  ai: boolean;
  selected: Set<string>;
  onToggle: (keys: string[], on: boolean) => void;
  alwaysShow: boolean;
}) {
  const [confirmReset, setConfirmReset] = useState(false);
  // Yapay zekâ yazıyor (düğme kilitli, dönen simge).
  const [writing, setWriting] = useState(false);
  const aiWrite = async (rewrite: boolean) => {
    setWriting(true);
    try {
      await run(() => aiWriteDays(sheet.id, [day.date], rewrite))();
    } finally {
      setWriting(false);
    }
  };
  // Satırın birim seçeneği: çizelgenin projelerinin birimleri ve dosyadaki birimler.
  const divisions = useMemo(() => {
    const list: string[] = [];
    const add = (d: string) => {
      const t = d.trim();
      if (t && !list.some((x) => x.toLocaleLowerCase("tr") === t.toLocaleLowerCase("tr"))) list.push(t);
    };
    for (const m of sheet.projects) add(m.division || projectName(projects, m.projectId) || "");
    for (const d of sheet.divisions) add(d);
    return list;
  }, [sheet, projects]);
  const date = parseIsoDate(day.date);
  const outsideHours = outside.reduce((s, r) => s + (r.hours ?? 0), 0);
  const total = day.entries.reduce((s, e) => s + e.hours, 0) + outsideHours;
  const totalActual = day.entries.reduce((s, e) => s + worked(e), 0);
  const exported = day.entries.length > 0 && day.entries.every((e) => e.exported);
  const empty =
    day.entries.length === 0 &&
    outside.length === 0 &&
    day.unassignedSeconds < 60 &&
    day.meetings.length === 0 &&
    day.hidden === 0;
  const weekend = date.getDay() === 0 || date.getDay() === 6;
  if (empty && weekend && !alwaysShow) return null;
  // Günlük saatten sapma (bugün ve öncesi; ileri tarihli gün henüz bitmedi).
  const diff = day.date <= isoDate(today()) ? hoursDiff(day, config.dayHours, outsideHours) : 0;
  // Kum'un satırları ve dosyada Kum dışında girilmiş satırlar, başlangıca göre (başlangıcı
  // olmayan dosya satırları sonda).
  const lines: ({ entry: EntryView; row?: undefined } | { row: SheetRowView; entry?: undefined })[] = [
    ...day.entries.map((entry) => ({ entry })),
    ...outside.map((row) => ({ row })),
  ];
  const startOf = (l: (typeof lines)[number]) => (l.entry ? l.entry.start : (l.row.start ?? "99"));
  lines.sort((a, b) => startOf(a).localeCompare(startOf(b)));
  const missing = day.entries.some(needsDetails);
  const open = day.entries.filter((e) => !e.exported);
  // Kaydedilmiş (düzenlenmiş, elle eklenmiş) ya da gizlenmiş satır varsa sıfırlanabilir.
  const touched = open.some((e) => e.id) || day.hidden > 0;
  const openKeys = open.map((e) => e.key);
  const chosen = openKeys.filter((k) => selected.has(k)).length;
  const sheetProjects = sheet.projects
    .map((m) => projects.find((p) => p.id === m.projectId))
    .filter((p): p is Tag => !!p);

  const addRow = (projectId: string) =>
    run(() =>
      api.saveTimesheetEntry(null, {
        date: day.date,
        start: "09:00:00",
        hours: 1,
        actualHours: null,
        kind: "Working",
        details: "",
        party: sheet.projects.find((m) => m.projectId === projectId)?.party || sheet.defaultParty,
        projectId,
        division: defaultDivision(sheet, projects, projectId),
        coverage: [],
      }),
    )();

  return (
    <section id={`gun-${day.date}`} className="scroll-mt-4 rounded-xl border bg-card shadow-xs">
      <div className="flex flex-wrap items-center gap-2 px-4 py-2.5">
        <span className="text-[13px] font-semibold capitalize">{dayFmt.format(date)}</span>
        <span className="text-xs text-muted-foreground tabular">
          {total ? manDays(total, config.dayHours) : "—"}
          {total > 0 && <span title="Takip edilen gerçek süre"> · gerçek {actual(totalActual)}</span>}
        </span>
        {exported && (
          <Badge variant="outline" className="border-success/40 text-success">
            Gönderildi
          </Badge>
        )}
        {diff !== 0 && (
          <Badge
            variant="outline"
            className={WARN_BADGE}
            title={`Günlük ${num.format(config.dayHours)} saatten ${diff < 0 ? "az" : "fazla"}`}
          >
            {signedHours(diff)}
          </Badge>
        )}
        {day.unassignedSeconds >= UNASSIGNED_MIN && (
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <button
              className="rounded underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onReviewDay(day.date)}
              title="Gözden geçir: atanmamış süreyi site ve uygulamaya göre projeye ata"
            >
              Atanmamış {formatDuration(day.unassignedSeconds)} · gözden geçir
            </button>
            <span aria-hidden>·</span>
            <button
              className="rounded underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onOpenDay(day.date)}
              title="Takvimde aç: blokları ya da aralıkları projeye ata"
            >
              takvim
            </button>
          </span>
        )}
        {day.hidden > 0 && (
          <button
            className="rounded text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            title="Silinen satırları geri getir"
            onClick={run(async () => {
              const ids = await api.restoreHiddenEntries(sheet.id, day.date);
              toast(`${ids.length} satır geri getirildi`, { tone: "success" });
            })}
          >
            {day.hidden} silinmiş · geri getir
          </button>
        )}
        <span className="ml-auto flex items-center gap-1.5">
          {missing && (
            <Button
              size="sm"
              variant="ghost"
              title="Önceki günlerin aynı projedeki açıklamalarını boş satırlara yaz"
              onClick={run(() => copyPreviousDetails(sheet.id, day.date))}
            >
              <Copy /> Önceki günden kopyala
            </Button>
          )}
          {ai && open.length > 0 && (
            <Button
              size="sm"
              variant="ghost"
              disabled={writing}
              title="Boş ya da otomatik gelen açıklamaları Claude yazar, elle yazdıklarına dokunulmaz. Bildirimden geri alınır."
              onClick={() => aiWrite(false)}
            >
              {writing ? <Loader2 className="animate-spin" /> : <Sparkles />} Yapay zekâyla yaz
            </Button>
          )}
          {ai && open.some((e) => e.details.trim()) && (
            <Button
              size="icon-sm"
              variant="ghost"
              disabled={writing}
              aria-label="Hepsini yapay zekâyla yeniden yaz"
              title="Hepsini yeniden yaz: elle yazılanlar dahil aktarılmamış bütün açıklamaları Claude yazar. Bildirimden geri alınır."
              onClick={() => aiWrite(true)}
            >
              <WandSparkles />
            </Button>
          )}
          {touched &&
            (confirmReset ? (
              <Button
                size="sm"
                variant="destructive"
                onClick={() => {
                  // Onay tek kullanımlık: sonraki tıklama yeniden onay istesin.
                  setConfirmReset(false);
                  run(() => api.resetTimesheetDay(sheet.id, day.date))();
                }}
              >
                Düzenlemeler silinsin, yeniden öner
              </Button>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setConfirmReset(true)}
                title="Düzenlemeleri, birleştirmeleri, elle eklenen ve silinen satırları geri al; satırlar takipten yeniden önerilir. Gönderilenlere dokunulmaz."
              >
                <RotateCcw /> Sıfırla
              </Button>
            ))}
          {sheetProjects.length === 1 && (
            <Button size="sm" variant="outline" onClick={() => addRow(sheetProjects[0].id)}>
              + Satır
            </Button>
          )}
          {sheetProjects.length > 1 && (
            <ProjectSelect
              value=""
              projects={sheetProjects}
              placeholder="+ Satır…"
              className="w-32"
              aria-label="Satır ekle: proje"
              onChange={addRow}
            />
          )}
        </span>
      </div>
      {lines.length > 0 && (
        // Dar pencerede açıklama sütunu ezilmesin: satırlar kart içinde yatay kayar.
        <div className="overflow-x-auto border-t">
          <div className={cn("grid items-center gap-2 px-4 pt-2 text-[11px] text-muted-foreground", ROW_GRID)}>
            {openKeys.length > 0 ? (
              <SelectBox
                checked={chosen === openKeys.length}
                indeterminate={chosen > 0 && chosen < openKeys.length}
                onChange={(on) => onToggle(openKeys, on)}
                label={`${dayFmt.format(date)}: gönderilmemiş satırların hepsini seç`}
              />
            ) : (
              <span />
            )}
            <span>Başlangıç</span>
            <span>Saat</span>
            <span>Tür</span>
            <span>Açıklama</span>
            <span>Taraf</span>
            <span>Birim</span>
            <span />
          </div>
          <ul className="pb-1.5">
            {lines.map(({ entry: e, row }) =>
              e ? (
                <EntryRow
                  key={e.key}
                  entry={e}
                  divisions={divisions}
                  projects={projects}
                  run={run}
                  runFile={runFile}
                  file={file}
                  sheets={!!sheet.sheetUrl}
                  sheetId={sheet.id}
                  selected={selected.has(e.key)}
                  onSelect={(on) => onToggle([e.key], on)}
                />
              ) : (
                <FileRowItem
                  key={`dosya-${row.row}-${row.start}-${row.details}`}
                  row={row}
                  sheet={sheet}
                  divisions={divisions}
                  runFile={runFile}
                />
              ),
            )}
          </ul>
        </div>
      )}
      {day.meetings.length > 0 && <MeetingList meetings={day.meetings} projects={projects} run={run} />}
    </section>
  );
}

/** Satır seçim kutusu (yarı seçili hali de olur). */
function SelectBox({
  checked,
  indeterminate = false,
  onChange,
  label,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (on: boolean) => void;
  label: string;
}) {
  return (
    <input
      type="checkbox"
      className="size-3.5 cursor-pointer accent-primary"
      checked={checked}
      ref={(el) => {
        if (el) el.indeterminate = indeterminate;
      }}
      onChange={(e) => onChange(e.target.checked)}
      aria-label={label}
    />
  );
}

/**
 * Takvimde olup hiçbir projeye düşmeyen toplantılar. Seçilen proje serinin tüm tekrarlarına
 * uygulanır (haftalık toplantı bir kez atanır); "Yoksay" seriyi zaman çizelgesinden çıkarır.
 */
function MeetingList(props: { meetings: UnassignedMeeting[]; projects: Tag[]; run: Run }) {
  return (
    <div className="border-t px-4 py-2">
      <div className="flex items-center gap-1.5 pb-1 text-[11px] text-muted-foreground">
        <CalendarDays className="size-3.5" /> Takvimden, projesi belli olmayan toplantılar
      </div>
      <MeetingRows {...props} />
    </div>
  );
}

/**
 * Toplantılar ve proje seçimi (gün kartında ve dönem denetiminde). Emin olunan öneri seçicinin
 * yanında "→ Proje" olarak durur (gerekçesi ipucunda); tıklayınca seri o projeye atanır.
 * Birden çok seri için öneri varsa hepsi tek düğmeyle atanır.
 */
function MeetingRows({ meetings, projects, run }: { meetings: UnassignedMeeting[]; projects: Tag[]; run: Run }) {
  const nameOf = (id: string) => projectName(projects, id);
  // Seri başına bir öneri (aynı gün iki tekrar olsa da bir kez atanır); adı bilinmeyen
  // (seçicide olmayan) proje önerilmez.
  const suggested = new Map<string, string>();
  for (const m of meetings) {
    if (m.suggestion && nameOf(m.suggestion.projectId)) suggested.set(m.uid, m.suggestion.projectId);
  }
  const assignAll = run(async () => {
    for (const [uid, projectId] of suggested) await api.assignMeeting(uid, projectId);
  });
  return (
    <ul className="space-y-1">
      {meetings.map((m) => {
        const s = m.suggestion;
        const name = s ? nameOf(s.projectId) : undefined;
        return (
          <li key={`${m.uid}-${m.start}`} className="flex flex-wrap items-center gap-2 text-xs">
            <span className="w-24 shrink-0 text-muted-foreground tabular">
              {timeFmt.format(new Date(m.start))}–{timeFmt.format(new Date(m.end))}
            </span>
            <span className="min-w-0 flex-1 truncate" title={m.location || undefined}>
              {m.subject || "(konusuz)"}
              <span className="ml-1.5 text-muted-foreground">{m.online ? "Online" : "F2F"}</span>
            </span>
            {s && name && (
              <button
                type="button"
                className="max-w-40 truncate rounded-md border border-dashed px-1.5 py-0.5 text-[11px] text-muted-foreground hover:border-solid hover:bg-accent hover:text-foreground"
                title={`Öneri: ${s.reason}`}
                aria-label={`${m.subject} toplantısını ${name} projesine ata (${s.reason})`}
                onClick={run(() => api.assignMeeting(m.uid, s.projectId))}
              >
                → {name}
              </button>
            )}
            <ProjectSelect
              value=""
              projects={projects}
              placeholder="Projeye ata…"
              extra={[{ value: IGNORE, label: "Zaman çizelgesine alma" }]}
              className="w-44"
              aria-label={`${m.subject} projesi`}
              onChange={(v) => run(() => api.assignMeeting(m.uid, v === IGNORE ? null : v))()}
            />
          </li>
        );
      })}
      {suggested.size > 1 && (
        <li className="flex justify-end">
          <button className={cn(FIX_LINK, "text-[11px]")} onClick={assignAll}>
            Önerilenleri ata ({suggested.size})
          </button>
        </li>
      )}
    </ul>
  );
}

function EntryRow({
  entry,
  divisions,
  projects,
  run,
  runFile,
  file,
  sheets,
  sheetId,
  selected,
  onSelect,
}: {
  entry: EntryView;
  /** Çizelgenin birimleri (satırın birimi bunlardan seçilir). */
  divisions: string[];
  projects: Tag[];
  run: Run;
  runFile: Run;
  file: FileState;
  /** Çizelge Google Sheets'e bağlı (değilse Excel). */
  sheets: boolean;
  sheetId: string;
  selected: boolean;
  onSelect: (on: boolean) => void;
}) {
  // Aktarılmış satır dosyadaki satırı bulunduysa düzenlenir; değişiklik dosyaya da yazılır.
  const fileRow = entry.exported && entry.id && file.kind === "ready" ? file.rowOf.get(entry.id) : undefined;
  const lost = entry.exported && !!entry.id && file.kind === "ready" && file.missing.has(entry.id);
  const editable = !entry.exported || fileRow !== undefined;
  const w = fileWords(sheets);
  const [draft, setDraft] = useState(entry);
  // Her yeniden yüklemede satırlar yeni nesne olarak gelir; yalnızca içerik değişince taslak
  // yenilenir ve henüz kaydedilmemiş yazılanlar yeni içeriğin üzerinde kalır (başka satırın
  // kaydı ya da takvim yenilemesi yazılanı silmesin).
  const base = useRef(entry);
  const entryJson = JSON.stringify(entry);
  useEffect(() => {
    const prev = base.current;
    const next: EntryView = JSON.parse(entryJson);
    base.current = next;
    setDraft((d) => {
      const out = { ...next };
      for (const k of EDITABLE) if (d[k] !== prev[k]) Object.assign(out, { [k]: d[k] });
      return out;
    });
  }, [entryJson]);
  // Canlı satır ilk düzenlemede kaydedilir (aralıklarıyla); sonra kimliğiyle güncellenir.
  // Aktarılmış satır önce dosyadaki satırına, sonra Kum'a yazılır.
  const save = (next: EntryView) =>
    entry.exported
      ? runFile(() => api.saveTimesheetEntry(entry.id, next, fileRow))()
      : run(() => api.saveTimesheetEntry(entry.id, next))();
  const commit = () => {
    const next = { ...draft, details: draft.details.trim(), party: draft.party.trim() };
    // Dosyadaki satırın açıklaması boşaltılmaz.
    if (entry.exported && !next.details) return setDraft({ ...draft, details: entry.details });
    if (EDITABLE.some((k) => next[k] !== entry[k])) save(next);
  };
  // Aktarılmış satır dosyadan da silinir; geri alınınca satır geri gelir ve yeniden gönderilir.
  const remove = () => {
    if (!entry.exported || !entry.id) return run(() => dismissRows([entry]))();
    const id = entry.id;
    return runFile(async () => {
      await api.dismissTimesheetEntry(id, entry, fileRow);
      toast(`Satır ${w.from} da silindi`, {
        action: {
          label: "Geri al",
          run: () =>
            runFile(async () => {
              await api.undismissTimesheetEntries([id]);
              await api.exportTimesheet(sheetId, [{ id, entry }]);
              toast(`Satır ${w.to} geri eklendi`, { tone: "success" });
            })(),
        },
      });
    })();
  };
  const project = projects.find((p) => p.id === entry.projectId);
  const options =
    divisions.some((d) => d === draft.division) || !draft.division ? divisions : [...divisions, draft.division];
  const cell = "h-7 px-1.5 text-xs";
  // Açıklaması boş satır gönderilemez: hafifçe vurgulanır.
  const missing = !entry.exported && !draft.details.trim();
  const stale = isStale(entry);
  const sentTitle =
    fileRow !== undefined
      ? `Gönderildi: ${w.of} ${fileRow}. satırı. Değişiklikler oraya da yazılır.`
      : lost
        ? `Gönderildi, ama ${w.in} bulunamadı (orada silinmiş ya da başlangıcı değiştirilmiş olabilir).`
        : file.kind === "loading"
          ? `Gönderildi; ${w.inAdj} satırı aranıyor…`
          : "Gönderildi";

  return (
    <li className={cn(stale && "bg-amber-500/5")}>
      <div className={cn("grid items-center gap-2 px-4 py-1", ROW_GRID, !editable && "text-muted-foreground")}>
        {!entry.exported ? (
          <SelectBox checked={selected} onChange={onSelect} label="Satırı seç" />
        ) : lost ? (
          <TriangleAlert className="size-3.5 text-amber-600 dark:text-amber-400" aria-label={sentTitle}>
            <title>{sentTitle}</title>
          </TriangleAlert>
        ) : (
          <Check className="size-3.5 text-success" aria-label={sentTitle}>
            <title>{sentTitle}</title>
          </Check>
        )}
        <Input
          type="time"
          className={cell}
          disabled={!editable}
          value={draft.start.slice(0, 5)}
          onChange={(e) => setDraft({ ...draft, start: `${e.target.value}:00` })}
          onBlur={commit}
          aria-label="Başlangıç"
        />
        <div className="flex items-center gap-1.5">
          <Input
            type="number"
            step="0.25"
            min="0.25"
            className={cn(cell, "w-16 tabular")}
            disabled={!editable}
            value={Number(draft.hours.toFixed(2))}
            onChange={(e) => setDraft({ ...draft, hours: Number(e.target.value) })}
            onBlur={commit}
            aria-label="Saat"
          />
          {draft.actualHours != null && (
            <span
              className="truncate text-[11px] text-muted-foreground tabular"
              title="Takip edilen gerçek süre; saat çeyreğe yuvarlanır"
            >
              {actual(draft.actualHours)}
            </span>
          )}
        </div>
        <Select value={draft.kind} disabled={!editable} onValueChange={(v) => save({ ...draft, kind: v as EntryKind })}>
          <SelectTrigger size="sm" className="h-7 text-xs" aria-label="Tür">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {KINDS.map((k) => (
              <SelectItem key={k} value={k}>
                {k}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          className={cn(cell, missing && "border-amber-500/50 bg-amber-500/5")}
          data-empty={missing || undefined}
          title={missing ? "Açıklama boş: bu satır gönderilemez" : undefined}
          disabled={!editable}
          list="timesheet-details"
          value={draft.details}
          placeholder={editable ? (draft.kind === "Working" ? "Ne yaptın?" : "Toplantı konusu?") : ""}
          onChange={(e) => setDraft({ ...draft, details: e.target.value })}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Açıklama"
        />
        <Input
          className={cell}
          disabled={!editable}
          list="timesheet-parties"
          value={draft.party}
          onChange={(e) => setDraft({ ...draft, party: e.target.value })}
          onBlur={commit}
          aria-label="Taraf"
        />
        <Select value={draft.division} disabled={!editable} onValueChange={(division) => save({ ...draft, division })}>
          <SelectTrigger
            size="sm"
            className="h-7 min-w-0 text-xs"
            aria-label="Birim"
            title={project ? `Proje: ${project.name}` : undefined}
          >
            <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} />
            <SelectValue placeholder="Birim seç" />
          </SelectTrigger>
          <SelectContent>
            {options.map((d) => (
              <SelectItem key={d} value={d}>
                {d}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {editable ? (
          <Button
            size="icon-sm"
            variant="ghost"
            className="size-7 text-muted-foreground hover:text-destructive"
            aria-label={entry.exported ? `Satırı ${w.from} da sil` : "Satırı sil"}
            title={
              entry.exported
                ? `Sil: satır ${w.from} da silinir; bildirimden geri alınır`
                : "Sil; bildirimden geri alınır"
            }
            onClick={remove}
          >
            <Trash2 />
          </Button>
        ) : (
          <span />
        )}
      </div>
      {stale && entry.id && (
        <div className="flex flex-wrap items-center gap-1.5 px-4 pb-1.5 pl-[42px] text-[11px] text-amber-700 dark:text-amber-400">
          <TriangleAlert className="size-3" />
          {entry.stale! > 0
            ? `Takipte değişti: işin bir kısmı raporda başka projeye alınmış, projede ${actual(entry.stale!)} kaldı.`
            : "Takipte değişti: bu satırın işi raporda başka projeye alınmış."}
          {entry.exported && ` Güncellenince ${w.inAdj} satırı da değişir.`}
          <button
            className="rounded font-medium underline underline-offset-2 hover:text-foreground disabled:opacity-50"
            disabled={entry.exported && fileRow === undefined}
            onClick={(entry.exported ? runFile : run)(() => refreshRows([entry.id!]))}
          >
            {entry.stale! > 0 ? "Güncelle" : "Kaldır"}
          </button>
        </div>
      )}
    </li>
  );
}

/** Dosya satırında düzenlenen alanlar. */
type FileDraft = Pick<FileRow, "start" | "hours" | "kind" | "details" | "party" | "division">;

/**
 * Dosyada Kum dışında girilmiş satır: Kum'da kaydı yok, doğrudan dosyadaki satır değişir.
 * Simgesinden tarihi değiştirilir (satır dosyada yeni gününe taşınır). Silme bildirimden geri
 * alınır; başlangıcı, saati ya da türü eksik satır geri eklenemeyeceği için iki tıklamayla silinir.
 */
function FileRowItem({
  row,
  sheet,
  divisions,
  runFile,
}: {
  row: SheetRowView;
  sheet: TimesheetInfo;
  divisions: string[];
  runFile: Run;
}) {
  const pick = (r: FileRow): FileDraft => ({
    start: r.start,
    hours: r.hours,
    kind: r.kind,
    details: r.details,
    party: r.party,
    division: r.division,
  });
  const [draft, setDraft] = useState<FileDraft>(() => pick(row));
  const rowJson = JSON.stringify(row);
  useEffect(() => setDraft(pick(JSON.parse(rowJson))), [rowJson]);
  const [confirmDelete, setConfirmDelete] = useState(false);
  useEffect(() => {
    if (!confirmDelete) return;
    const t = setTimeout(() => setConfirmDelete(false), 4000);
    return () => clearTimeout(t);
  }, [confirmDelete]);
  const w = fileWords(!!sheet.sheetUrl);
  const clean = (next: FileDraft) => ({ ...next, details: next.details.trim(), party: next.party.trim() });
  const save = (next: FileDraft) => {
    const c = clean(next);
    const changed = (Object.keys(c) as (keyof FileDraft)[]).some((k) => c[k] !== row[k]);
    if (!changed || !c.start || !c.hours) return;
    runFile(() => api.saveSheetRow(sheet.id, row, { ...row, ...c }))();
  };
  // Dosyaya yeniden yazılabilir: başlangıcı, saati ve türü geçerli (geri ekleme, taşıma).
  const complete = (r: FileDraft) => !!r.start && !!r.hours && KINDS.includes(r.kind as EntryKind);
  const restorable = complete(row);
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveTo, setMoveTo] = useState(row.date);
  /** Satırı başka güne taşır; bildirimden geri alınır (eski gününe döner). */
  const move = () => {
    const date = moveTo;
    if (!date || date === row.date) return;
    setMoveOpen(false);
    const next: FileRow = { ...row, ...clean(draft), date };
    runFile(async () => {
      const at = await api.saveSheetRow(sheet.id, row, next);
      toast(`Satır ${dayFmt.format(parseIsoDate(date))} gününe taşındı`, {
        tone: "success",
        action: {
          label: "Geri al",
          run: () => runFile(() => api.saveSheetRow(sheet.id, { ...next, row: at }, row))(),
        },
      });
    })();
  };
  const remove = () => {
    if (!restorable && !confirmDelete) return setConfirmDelete(true);
    setConfirmDelete(false);
    runFile(async () => {
      await api.deleteSheetRow(sheet.id, row);
      toast(
        `Satır ${w.from} silindi`,
        restorable
          ? {
              action: {
                label: "Geri al",
                run: () =>
                  runFile(async () => {
                    await api.restoreSheetRow(sheet.id, row);
                    toast(`Satır ${w.to} geri eklendi`, { tone: "success" });
                  })(),
              },
            }
          : { tone: "success" },
      );
    })();
  };
  const kinds = KINDS.includes(draft.kind as EntryKind) || !draft.kind ? KINDS : [...KINDS, draft.kind];
  const options =
    divisions.some((d) => d === draft.division) || !draft.division ? divisions : [...divisions, draft.division];
  const cell = "h-7 px-1.5 text-xs";
  const title = `${w.Of} ${row.row}. satırı; Kum dışında girilmiş. Değişiklikler doğrudan oraya yazılır. Tıkla: tarihi değiştir.`;
  const Icon = sheet.sheetUrl ? Sheet : FileSpreadsheet;

  return (
    <li className="bg-muted/30">
      <div className={cn("grid items-center gap-2 px-4 py-1", ROW_GRID)}>
        <Popover
          open={moveOpen}
          onOpenChange={(o) => {
            setMoveOpen(o);
            if (o) setMoveTo(row.date);
          }}
        >
          <PopoverTrigger asChild>
            <button
              className="-m-1 grid size-6 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
              title={title}
              aria-label={`${w.Of} ${row.row}. satırı: tarihi değiştir`}
            >
              <Icon className="size-3.5" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-64 space-y-2.5 p-3">
            <p className="text-xs text-muted-foreground">
              {w.Of} {row.row}. satırı · Kum dışında girilmiş
            </p>
            <label className="block space-y-1 text-xs font-medium">
              <span>Tarih</span>
              <Input
                type="date"
                className="h-8 text-xs"
                value={moveTo}
                onChange={(e) => setMoveTo(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && move()}
              />
            </label>
            {!complete(draft) && (
              <p className="text-[11px] text-amber-700 dark:text-amber-400">
                Taşımak için başlangıç, saat ve tür dolu olmalı.
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="ghost" onClick={() => setMoveOpen(false)}>
                Vazgeç
              </Button>
              <Button size="sm" disabled={!moveTo || moveTo === row.date || !complete(draft)} onClick={move}>
                Taşı
              </Button>
            </div>
          </PopoverContent>
        </Popover>
        <Input
          type="time"
          className={cell}
          value={draft.start?.slice(0, 5) ?? ""}
          onChange={(e) => setDraft({ ...draft, start: e.target.value ? `${e.target.value}:00` : null })}
          onBlur={() => save(draft)}
          aria-label="Başlangıç"
        />
        <Input
          type="number"
          step="0.25"
          min="0.25"
          className={cn(cell, "w-16 tabular")}
          value={draft.hours == null ? "" : Number(draft.hours.toFixed(2))}
          onChange={(e) => setDraft({ ...draft, hours: e.target.value === "" ? null : Number(e.target.value) })}
          onBlur={() => save(draft)}
          aria-label="Saat"
        />
        <Select value={draft.kind} onValueChange={(kind) => save({ ...draft, kind })}>
          <SelectTrigger size="sm" className="h-7 text-xs" aria-label="Tür">
            <SelectValue placeholder="Tür" />
          </SelectTrigger>
          <SelectContent>
            {kinds.map((k) => (
              <SelectItem key={k} value={k}>
                {k}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          className={cell}
          list="timesheet-details"
          value={draft.details}
          onChange={(e) => setDraft({ ...draft, details: e.target.value })}
          onBlur={() => save(draft)}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Açıklama"
        />
        <Input
          className={cell}
          list="timesheet-parties"
          value={draft.party}
          onChange={(e) => setDraft({ ...draft, party: e.target.value })}
          onBlur={() => save(draft)}
          aria-label="Taraf"
        />
        <Select value={draft.division} onValueChange={(division) => save({ ...draft, division })}>
          <SelectTrigger size="sm" className="h-7 min-w-0 text-xs" aria-label="Birim">
            <SelectValue placeholder="Birim seç" />
          </SelectTrigger>
          <SelectContent>
            {options.map((d) => (
              <SelectItem key={d} value={d}>
                {d}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          size="icon-sm"
          variant={confirmDelete ? "destructive" : "ghost"}
          className={cn("size-7", !confirmDelete && "text-muted-foreground hover:text-destructive")}
          aria-label={confirmDelete ? `Satırı ${w.from} sil: onayla` : `Satırı ${w.from} sil`}
          title={
            restorable
              ? `Sil: satır ${w.from} silinir; bildirimden geri alınır`
              : confirmDelete
                ? `Onaylamak için tekrar tıkla: satır ${w.from} silinir`
                : `Sil: başlangıcı, saati ya da türü eksik satır geri eklenemez; iki tıklamayla silinir`
          }
          onClick={remove}
        >
          {confirmDelete ? <Check /> : <Trash2 />}
        </Button>
      </div>
    </li>
  );
}

/**
 * İlk kurulum: kayıtların yazılacağı dosya (Excel ya da Google Sheets) ve isteğe bağlı Outlook
 * takvimi. Sonradan Ayarlar'dan değiştirilir; başka firmalar için yeni çizelge eklenir.
 */
function Setup({ onDone }: { onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [sheets, setSheets] = useState(false);
  const step = "space-y-2.5 rounded-xl border bg-card px-5 py-4 shadow-xs";
  return (
    <div className="mx-auto w-full max-w-xl space-y-4 px-6 pt-8 pb-10">
      <div className="space-y-1.5 px-1">
        <h1 className="text-base font-semibold">Zaman çizelgesi</h1>
        <p className="text-sm text-muted-foreground">
          Projeye atanmış çalışma süren ve takvimindeki toplantılar günlük iş kayıtlarına dönüşür; gönderdiğin kayıtlar
          firmanın dosyasına aynı sütun ve biçimle eklenir. Her firmanın çizelgesine yalnızca ona bağladığın projelerin
          işi gider.
        </p>
      </div>
      <section className={step}>
        <h2 className="text-[13px] font-semibold">1. Kayıtlar nereye yazılsın?</h2>
        <p className="text-xs text-muted-foreground">
          Firma, danışman, birimler (projeler) ve geçmiş açıklamalar seçtiğin dosyadan alınır.
        </p>
        {sheets ? (
          <SheetConnect timesheetId={null} onDone={onDone} onCancel={() => setSheets(false)} />
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  const path = await api.pickTimesheetFile();
                  if (path) {
                    await api.importTimesheetTemplate(null, path);
                    onDone();
                  }
                } catch (e) {
                  setError(friendlyError(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <FileSpreadsheet /> Excel dosyası seç
            </Button>
            <Button variant="outline" onClick={() => setSheets(true)}>
              <Sheet /> Google Sheets'e bağla
            </Button>
          </div>
        )}
        <ErrorText>{error}</ErrorText>
      </section>
      <section className={step}>
        <h2 className="text-[13px] font-semibold">
          2. Outlook takvimi <span className="font-normal text-muted-foreground">(isteğe bağlı)</span>
        </h2>
        <p className="text-xs text-muted-foreground">Toplantılar da kayıtlara girer.</p>
        <CalendarConnect />
      </section>
      <p className="px-1 text-xs text-muted-foreground">
        İkisini de sonra <b>Ayarlar</b>'dan değiştirebilirsin.
      </p>
    </div>
  );
}
