import { useCallback, useEffect, useRef, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronLeft,
  ChevronRight,
  ClipboardCheck,
  FileSpreadsheet,
  FolderKanban,
  Loader2,
  RefreshCw,
  Settings2,
  Sheet,
  X,
} from "lucide-react";
import {
  api,
  type CalendarStatus,
  type EntryView,
  type Exported,
  type SheetRows,
  type SheetRowView,
  type Tag,
  type TimesheetConfig,
  type TimesheetDay,
} from "../api";
import { useTauriEvent } from "../lib/useTauriEvent";
import { CONNECTIONS_SECTION, fileName, TIMESHEET_SECTION } from "./TimesheetSettings";
import { ErrorText } from "../components/settings";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { addDays, addMonths, daysInMonth, isoDate, parseIsoDate, today } from "../lib/dates";
import { MonthBoard, WeekBoard } from "../components/TimesheetBoard";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { friendlyError, useChanged } from "../lib/feedback";
import { blocked, closeReport, divisionColor, keepRowIds, started, summarizeDay } from "../lib/timesheet";
import {
  num,
  exportNotice,
  type Mode,
  MODES,
  MODE_KEY,
  SHEET_KEY,
  savedMode,
  savedSheet,
  range,
  rangeTitle,
  actual,
  worked,
  toRef,
  defaultDivision,
  showDay,
  RAIL,
  RAIL_TITLE,
  SOURCE,
  Page,
  Stat,
  type Run,
  useBusy,
} from "./timesheet/shared";
import { dismissRows, mergeRows } from "./timesheet/actions";
import {
  type FileState,
  FILE_FRESH_MS,
  readFileCache,
  mergeFileParts,
  writeFileCache,
  FileError,
} from "./timesheet/fileRows";
import { SelectionBar } from "./timesheet/SelectionBar";
import { ProjectsPrompt, Setup } from "./timesheet/Setup";
import { ClosePanel } from "./timesheet/ClosePanel";
import { DayCard } from "./timesheet/DayCard";

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
  const [merging, guardMerge] = useBusy();
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

  // Çizelgenin dosyasındaki satırlar: hangi çizelge ve ay aralığının olduğu, okunuyor mu,
  // okunamadıysa neden.
  const [file, setFile] = useState<{ of: string; data: SheetRows } | null>(null);
  // Dosyaya yazılmakta olan değişiklik sayısı (Sheets yanıtı saniyeler sürer; arka planda yazılır).
  const [writing, setWriting] = useState(0);
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
  // Dosya ay ay okunur ve saklanır: hafta değiştirmek ya da ay sınırını geçmek yalnızca henüz
  // okunmamış (ya da eskimiş) ayı ister.
  const fileBase =
    fileSheet && (fileSheet.sheetUrl || fileSheet.filePath)
      ? `${fileSheet.id}|${fileSheet.sheetUrl ?? fileSheet.filePath}`
      : null;
  const months = [...new Set([start.slice(0, 7), isoDate(addDays(rangeStart, rangeDays - 1)).slice(0, 7)])];
  const fileOf = fileBase ? `${fileBase}|${months.join(",")}` : null;
  /**
   * Dosyanın satırlarını okur. Önbellekteki aylar hemen gösterilir; `force` değilse yalnızca
   * olmayan ya da eskimiş aylar istenir. Dosyada değiştirilen satırlar Kum'a geçtiyse günler
   * yeniden yüklenir.
   */
  const loadFile = useCallback(
    async (force = false) => {
      // Yeni çağrı uçuştaki okumayı geçersiz kılar; okuma başlatmadan dönerse yükleniyor
      // durumunu da kapatmalı, yoksa geçersiz kalan okuma onu hiç kapatmaz.
      const seq = ++fileSeq.current;
      if (!fileOf || !fileBase || !fileSheet) {
        setFileLoading(false);
        return;
      }
      const ms = fileOf.split("|").pop()!.split(",");
      const show = () => {
        const parts = ms.map((m) => readFileCache(`${fileBase}|${m}`));
        if (parts.every(Boolean)) {
          const data = mergeFileParts(parts.map((p) => p!.data));
          // Yeniden okunan satırlar ekrandaki kimliklerini korur (düzenlenen satır kurulmasın).
          setFile((cur) => ({ of: fileOf, data: { ...data, rows: keepRowIds(cur?.data.rows ?? [], data.rows) } }));
        }
        return parts;
      };
      const parts = show();
      const need = ms.filter((_, i) => force || !parts[i] || Date.now() - parts[i]!.at >= FILE_FRESH_MS);
      if (need.length === 0) {
        setFileLoading(false);
        return;
      }
      setFileLoading(true);
      try {
        const from = parseIsoDate(`${need[0]}-01`);
        const last = parseIsoDate(`${need[need.length - 1]}-01`);
        const days = Math.round((+addDays(last, daysInMonth(last)) - +from) / 86_400_000);
        const data = await api.sheetRows(fileSheet.id, isoDate(from), days);
        if (seq !== fileSeq.current) return;
        for (const m of need)
          writeFileCache(`${fileBase}|${m}`, {
            rows: data.rows.filter((r) => r.date.startsWith(m)),
            missing: data.missing,
            synced: 0,
          });
        show();
        setFileError(null);
        if (data.synced > 0) await load();
      } catch (e) {
        if (seq === fileSeq.current) setFileError({ of: fileOf, message: friendlyError(e) });
      } finally {
        if (seq === fileSeq.current) setFileLoading(false);
      }
    },
    // `fileSheet` her yüklemede yeni nesne: kimliği ve dosyası `fileBase`'te.
    [fileOf, fileBase, load],
  );
  useEffect(() => {
    loadFile();
  }, [loadFile]);
  /**
   * Yazılan değişikliği ekrandaki (ve önbellekteki) dosya satırlarına uygular; dosya yeniden
   * okunmaz. Satır numaraları kayabilir: yazarken yalnızca ipucudur, satır içeriğinden bulunur.
   */
  const patchFile = (f: (rows: SheetRowView[]) => SheetRowView[]) =>
    setFile((cur) => {
      if (!cur) return cur;
      const data = { ...cur.data, rows: keepRowIds(cur.data.rows, f(cur.data.rows)) };
      const [id, target, list] = cur.of.split("|");
      for (const m of list.split(",")) {
        const key = `${id}|${target}|${m}`;
        const old = readFileCache(key);
        writeFileCache(
          key,
          { rows: data.rows.filter((r) => r.date.startsWith(m)), missing: old?.data.missing ?? [], synced: 0 },
          old?.at,
        );
      }
      return { of: cur.of, data };
    });
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
  /**
   * Dosyaya da yazan işlemler: beklenmez (Sheets yanıtı saniyeler sürer), arka planda sırayla
   * yazılır; yazılırken üstte "tabloya yazılıyor" görünür. Ekrandaki satırları `f` kendisi
   * günceller ([patchFile]); dosya yeniden okunmaz.
   */
  const runFile: Run = (f) => async () => {
    setWriting((n) => n + 1);
    try {
      await run(f)();
    } finally {
      setWriting((n) => n - 1);
    }
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
      void loadFile(true);
    }
  };

  if (!config)
    return (
      <Page title="Zaman çizelgesi">
        {error ? (
          <div className="px-5">
            <ErrorText>{error}</ErrorText>
          </div>
        ) : (
          <div className="mx-auto w-full max-w-[1400px] space-y-3 px-5 pt-1 pb-10" aria-busy>
            {[0, 1, 2].map((i) => (
              <div key={i} className="skeleton h-28 rounded-xl" style={{ animationDelay: `${i * 120}ms` }} />
            ))}
          </div>
        )}
      </Page>
    );
  if (!sheet)
    return (
      <Page title="Zaman çizelgesi">
        <Setup onDone={load} />
      </Page>
    );
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
  // Dosya ay ay okunur: yalnızca ekrandaki günlerin satırları.
  const shownDates = new Set(days.map((d) => d.date));
  const fileRows = (fileData?.rows ?? []).filter((r) => shownDates.has(r.date));
  const missing = new Set(fileData?.missing ?? []);
  const outside = fileRows.filter((r) => !r.entryId);
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
      patchFile={patchFile}
      reloadFile={() => void loadFile(true)}
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
  const exportedRows = all.filter((e) => e.exported).length;
  const divisionTotals = [...byDivision.entries()].sort((a, b) => b[1] - a[1]);
  const divisionMax = Math.max(0, ...divisionTotals.map(([, h]) => h));
  const fileProblem = fileError?.of === fileOf ? fileError.message : null;
  const fileMissing = all.filter((e) => e.id && missing.has(e.id)).length;
  const where = sheet.sheetUrl ? "Tablodaki" : "Excel dosyasındaki";

  // Rapor sayfalarındaki gibi: başlık solda, görünüm ve gezinti sağda. Etiketler değişse de
  // düğmeler yerinden oynamaz.
  const controls = (
    <>
      <Tabs value={mode} onValueChange={(v) => setMode(v as Mode)}>
        <TabsList aria-label="Görünüm">
          {MODES.map((m) => (
            <TabsTrigger key={m.id} value={m.id} className="px-3.5">
              {m.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <div className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Önceki ${modeInfo.label.toLowerCase()}`}
          title="Önceki (←)"
          onClick={() => step(-1)}
        >
          <ChevronLeft />
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="w-[4.75rem]"
          disabled={start === current}
          title="Bugüne dön (T)"
          onClick={() => setAnchor(isoDate(today()))}
        >
          {modeInfo.current}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Sonraki ${modeInfo.label.toLowerCase()}`}
          title="Sonraki (→)"
          disabled={start >= current}
          onClick={() => step(1)}
        >
          <ChevronRight />
        </Button>
      </div>
    </>
  );

  return (
    <Page title={rangeTitle(mode, rangeStart)} controls={controls}>
      <div className="mx-auto grid w-full max-w-[1400px] items-start gap-4 px-5 pt-1 pb-10 @[1060px]:grid-cols-[minmax(0,1fr)_256px]">
        {/* Dönemin özeti, durumu ve kaynakları. Dar pencerede satırlar ezilmesin diye panel üstte
            yan yana üç bölüm olur. */}
        <aside
          aria-label="Dönem özeti"
          className="grid gap-px overflow-hidden rounded-xl border bg-border shadow-xs @[640px]:grid-cols-3 @[1060px]:sticky @[1060px]:top-1 @[1060px]:order-last @[1060px]:grid-cols-1"
        >
          <section className={RAIL}>
            <div className="-mr-1.5 flex items-center gap-1.5">
              {config.timesheets.length > 1 ? (
                <Select value={sheet.id} onValueChange={setSheetId}>
                  <SelectTrigger size="sm" className="min-w-0 flex-1 font-semibold" aria-label="Zaman çizelgesi">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {config.timesheets.map((t) => (
                      <SelectItem key={t.id} value={t.id}>
                        {t.company || "Adsız çizelge"}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <span className="min-w-0 flex-1 truncate text-[13px] font-semibold">
                  {sheet.company || "Zaman çizelgesi"}
                </span>
              )}
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Çizelge ayarları"
                title="Ayarlar → Zaman çizelgeleri"
                onClick={() => onOpenSettings(TIMESHEET_SECTION)}
              >
                <Settings2 />
              </Button>
            </div>
            <div>
              <div className="text-2xl leading-tight font-semibold tracking-tight tabular">
                {num.format(total)}{" "}
                <span className="text-[13px] font-medium tracking-normal text-muted-foreground">
                  sa · {num.format(total / (config.dayHours || 8))} ag
                </span>
              </div>
              {all.length > 0 && (
                <p className="text-xs text-muted-foreground tabular" title="Takip edilen gerçek süre">
                  gerçek {actual(totalActual)}
                </p>
              )}
            </div>
            {divisionTotals.length > 0 && (
              <ul className="space-y-1.5" aria-label="Birimlere dağılım">
                {divisionTotals.map(([d, h]) => (
                  <li key={d} className="space-y-1 text-xs">
                    <div className="flex items-center gap-2">
                      <i
                        className="size-2 shrink-0 rounded-full"
                        style={{ background: divisionColor(divisionList, d) }}
                      />
                      <span className="min-w-0 flex-1 truncate" title={d}>
                        {d || "(birimsiz)"}
                      </span>
                      <span className="font-medium tabular">{num.format(h)} sa</span>
                    </div>
                    <div className="h-1 overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full"
                        style={{
                          width: `${divisionMax ? (h / divisionMax) * 100 : 0}%`,
                          background: divisionColor(divisionList, d),
                        }}
                      />
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className={RAIL}>
            <h2 className={RAIL_TITLE}>Durum</h2>
            <div className="grid grid-cols-3 gap-1.5">
              <Stat
                value={fresh ? report.issues : null}
                label="sorun"
                tone="bg-amber-500/10 text-amber-700 dark:text-amber-400"
                title="Göndermeden önce bakılacaklar (dönemi denetle)"
              />
              <Stat
                value={pending.length}
                label="bekliyor"
                tone="bg-primary/10 text-primary"
                title={later ? `Henüz başlamamış ${later} satır hariç` : "Gönderilmemiş satırlar"}
              />
              <Stat value={exportedRows} label="gönderildi" tone="bg-success/10 text-success" />
            </div>
            <Button
              className="w-full"
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
            <Button
              variant={closing ? "secondary" : "outline"}
              className="w-full"
              aria-expanded={closing}
              onClick={() => setClosing((v) => !v)}
              title="Göndermeden önce dönemi denetle: atanmamış süre, toplantılar, saatler, açıklamalar"
            >
              <ClipboardCheck /> {modeInfo.close}
            </Button>
            {notice && (
              <div className="space-y-1 rounded-lg border border-success/30 bg-success/10 px-2.5 py-2 text-xs">
                <div className="flex items-start gap-1.5">
                  <Check className="mt-0.5 size-3.5 shrink-0 text-success" />
                  <span className="min-w-0 flex-1 break-words selectable">{notice}</span>
                  <button
                    aria-label="Kapat"
                    className="-m-1 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                    onClick={() => setNotice(null)}
                  >
                    <X className="size-3.5" />
                  </button>
                </div>
                {undoable && (
                  <button
                    disabled={undoing}
                    className="ml-5 font-medium underline-offset-2 hover:underline disabled:opacity-50"
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
                        void loadFile(true);
                      }
                    }}
                  >
                    {undoing ? "Geri alınıyor…" : "Aktarımı geri al"}
                  </button>
                )}
              </div>
            )}
          </section>

          {/* Kayıtların nereden gelip nereye gittiği; tıklayınca Ayarlar. */}
          <section className={RAIL}>
            <h2 className={RAIL_TITLE}>Kaynaklar</h2>
            <ul className="space-y-2.5 text-xs">
              <li>
                <button
                  className={SOURCE}
                  onClick={() => onOpenSettings(TIMESHEET_SECTION)}
                  title="Bu çizelgeye yalnızca bu projelerin işi gider (Ayarlar → Zaman çizelgeleri)"
                >
                  <FolderKanban className="mt-px size-3.5 shrink-0" />
                  {sheetProjects.length ? (
                    <span className="flex min-w-0 flex-wrap gap-x-2 gap-y-0.5">
                      {sheetProjects.map((p) => (
                        <span key={p.id} className="flex min-w-0 items-center gap-1">
                          <i className="size-1.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
                          <span className="truncate">{p.name}</span>
                        </span>
                      ))}
                    </span>
                  ) : (
                    <span className="text-amber-700 dark:text-amber-400">Proje seçilmedi</span>
                  )}
                </button>
              </li>
              <li className="space-y-0.5">
                <button
                  className={cn(SOURCE, !fileOf && "text-amber-700 dark:text-amber-400")}
                  onClick={() => onOpenSettings(TIMESHEET_SECTION)}
                  title="Ayarlar → Zaman çizelgeleri"
                >
                  {sheet.sheetUrl ? (
                    <Sheet className="mt-px size-3.5 shrink-0" />
                  ) : (
                    <FileSpreadsheet className="mt-px size-3.5 shrink-0" />
                  )}
                  <span className="truncate">{target}</span>
                </button>
                {fileOf && (
                  <div className="flex items-center gap-1.5 pl-5.5 text-[11px] text-muted-foreground">
                    {fileProblem ? (
                      <span className="text-amber-700 dark:text-amber-400">satırlar okunamadı</span>
                    ) : !fileData ? (
                      <span className="flex items-center gap-1" title="Bu sırada Kum'un satırlarıyla çalışabilirsin">
                        <Loader2 className="size-3 animate-spin" /> satırlar okunuyor…
                      </span>
                    ) : (
                      <span className="tabular">
                        {where.toLocaleLowerCase("tr")} {fileRows.length} satır
                        {outside.length > 0 && ` · ${outside.length} Kum dışında`}
                        {fileMissing > 0 && (
                          <span
                            className="text-amber-700 dark:text-amber-400"
                            title={`Gönderilen ${fileMissing} satır dosyada bulunamadı`}
                          >
                            {` · ${fileMissing} bulunamadı`}
                          </span>
                        )}
                      </span>
                    )}
                    {(fileData || fileProblem) && (
                      <button
                        className="ml-auto rounded p-0.5 hover:bg-accent hover:text-foreground disabled:opacity-50"
                        disabled={fileLoading}
                        onClick={() => void loadFile(true)}
                        aria-label={`${where} satırları yeniden oku`}
                        title={`${where} satırları yeniden oku`}
                      >
                        {fileLoading ? <Loader2 className="size-3 animate-spin" /> : <RefreshCw className="size-3" />}
                      </button>
                    )}
                  </div>
                )}
                {writing > 0 && (
                  <div className="flex items-center gap-1 pl-5.5 text-[11px] text-primary" role="status">
                    <Loader2 className="size-3 animate-spin" />
                    {sheet.sheetUrl ? "tabloya" : "Excel dosyasına"} yazılıyor{writing > 1 ? ` (${writing})` : ""}…
                  </div>
                )}
              </li>
              <li>
                <button
                  className={SOURCE}
                  onClick={() => onOpenSettings(CONNECTIONS_SECTION)}
                  title="Ayarlar → Bağlantılar"
                >
                  <CalendarDays className="mt-px size-3.5 shrink-0" />
                  <span className={cn(calendar?.last && !calendar.last.ok && "text-destructive")}>{calendarText}</span>
                </button>
              </li>
            </ul>
          </section>
        </aside>

        <div className="min-w-0 space-y-4">
          <ErrorText>{error}</ErrorText>
          {fileOf && fileProblem && (
            <FileError
              sheets={!!sheet.sheetUrl}
              loading={fileLoading}
              error={fileProblem}
              onReload={() => void loadFile(true)}
            />
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
              // Günler eskiyse (dönem değişiyor, yükleniyor) gönderilmez; üstteki düğme gibi.
              busy={exporting || !fresh}
              merging={merging}
              onClear={() => setSelected(new Set())}
              onMerge={guardMerge(run(() => mergeRows(selectedRows).then(() => setSelected(new Set()))))}
              onSend={() => send(selectedRows)}
              onDismiss={run(() => dismissRows(selectedRows).then(() => setSelected(new Set())))}
            />
          )}
        </div>
      </div>
    </Page>
  );
}
