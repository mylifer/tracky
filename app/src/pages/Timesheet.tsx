import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronLeft,
  ChevronRight,
  FileSpreadsheet,
  Plus,
  RefreshCw,
  Settings2,
  Sheet,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  formatDuration,
  type CalendarStatus,
  type EntryKind,
  type EntryView,
  type Exported,
  type Meeting,
  type Tag,
  type TimesheetConfig,
  type TimesheetDay,
  type TimesheetEntry,
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
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

const KINDS: EntryKind[] = ["Working", "Online", "F2F"];
const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric", month: "short" });
const num = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });
/** Takvimde "yoksay" seçeneğinin değeri. */
const IGNORE = "__yoksay__";

function exportNotice(r: Exported) {
  const where = r.sheets ? `Google Sheets'e (${r.target})` : "Excel'e";
  const skipped = r.skipped ? `, ${r.skipped} satır zaten yazılmıştı` : "";
  const backup = r.backup ? ` Yedek: ${r.backup}` : "";
  return `${r.rows} satır ${where} eklendi (${r.filled} boş satıra, ${r.inserted} yeni satır${skipped}).${backup}`;
}

type Mode = "day" | "week" | "month";
const MODES: { id: Mode; label: string; current: string }[] = [
  { id: "day", label: "Gün", current: "Bugün" },
  { id: "week", label: "Hafta", current: "Bu hafta" },
  { id: "month", label: "Ay", current: "Bu ay" },
];
const MODE_KEY = "kum.timesheet.mode";
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

/**
 * Zaman çizelgesi: haftanın günleri için projeye atanmış süreden önerilen iş kayıtları.
 * Gün onaylanınca satırlar düzenlenebilir; onaylı ve aktarılmamış satırlar Excel dosyasına eklenir.
 */
export default function Timesheet({
  onOpenDay,
  onOpenSettings,
}: {
  onOpenDay: (iso: string) => void;
  /** Ayarlar'ı bu bölümle aç (Bağlantılar ya da Zaman çizelgesi). */
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
  const { start: rangeStart, days: rangeDays } = range(mode, parseIsoDate(anchor));
  const start = isoDate(rangeStart);
  const [config, setConfig] = useState<TimesheetConfig | null>(null);
  const [days, setDays] = useState<TimesheetDay[]>([]);
  const [details, setDetails] = useState<string[]>([]);
  const [projects, setProjects] = useState<Tag[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Aktarım sürerken düğme kilitli: çift tıklama aynı satırları dosyaya iki kez yazmasın.
  const [exporting, setExporting] = useState(false);
  const [calendar, setCalendar] = useState<CalendarStatus | null>(null);

  // Hafta hızla değiştirilince geç gelen eski yanıt yenisinin üzerine yazmasın.
  const loadSeq = useRef(0);
  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    try {
      const [c, d, det, tax] = await Promise.all([
        api.timesheetConfig(),
        api.timesheetDays(start, rangeDays),
        api.timesheetDetails(),
        api.taxonomy(),
      ]);
      if (seq !== loadSeq.current) return;
      setConfig(c);
      setDays(d);
      setDetails(det);
      setProjects(tax.tags.filter((t) => t.kind === "project"));
      setError(null);
    } catch (e) {
      if (seq === loadSeq.current) setError(String(e));
    }
  }, [start, rangeDays]);
  useEffect(() => {
    load();
    api.calendarStatus().then(setCalendar, () => {});
  }, [load]);
  // Takvim arka planda yenilenince toplantılar değişmiş olabilir.
  useTauriEvent(api.onCalendar, (s) => {
    setCalendar(s);
    load();
  });

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  if (!config) return <ErrorText>{error}</ErrorText>;
  if (!config.filePath && !config.sheetUrl) return <Setup onDone={load} />;
  const target = config.sheetUrl ? "Google Sheets" : `Excel · ${fileName(config.filePath ?? "")}`;
  const calendarText = !calendar?.url
    ? "Outlook takvimi bağlı değil"
    : calendar.last && !calendar.last.ok
      ? "Outlook takvimi okunamadı"
      : "Outlook takvimi bağlı";

  const all = days.flatMap((d) => d.entries);
  const pending = days.filter((d) => d.approved).flatMap((d) => d.entries.filter((e) => !e.exported));
  const total = all.reduce((s, e) => s + e.hours, 0);
  const totalActual = all.reduce((s, e) => s + worked(e), 0);
  const byDivision = new Map<string, number>();
  for (const e of all) byDivision.set(e.division, (byDivision.get(e.division) ?? 0) + e.hours);
  const current = isoDate(range(mode, today()).start);
  const step = (n: number) => {
    const a = parseIsoDate(start);
    setAnchor(isoDate(mode === "day" ? addDays(a, n) : mode === "week" ? addDays(a, 7 * n) : addMonths(a, n)));
  };
  const modeInfo = MODES.find((m) => m.id === mode)!;

  return (
    <div className="mx-auto w-full max-w-5xl space-y-4 px-6 pt-2 pb-10">
      <div className="flex flex-wrap items-center gap-2">
        <div className="mr-auto">
          <h1 className="text-[15px] font-semibold">
            {config.company || "Zaman çizelgesi"} · {rangeTitle(mode, rangeStart)}
          </h1>
          <p className="text-xs text-muted-foreground">
            Toplam {manDays(total, config.dayHours)}
            {all.length > 0 && <span title="Takip edilen gerçek süre"> (gerçek {actual(totalActual)})</span>}
            {[...byDivision.entries()].map(([d, h]) => ` · ${d}: ${num.format(h)} sa`).join("")}
          </p>
          {/* Kayıtların nereden gelip nereye gittiği; tıklayınca Ayarlar → Bağlantılar. */}
          <button
            className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            onClick={() => onOpenSettings(CONNECTIONS_SECTION)}
            title="Ayarlar → Bağlantılar"
          >
            {config.sheetUrl ? <Sheet className="size-3" /> : <FileSpreadsheet className="size-3" />}
            <span className="max-w-60 truncate">{target}</span>
            <span aria-hidden>·</span>
            <CalendarDays className="size-3" />
            <span className={cn(calendar?.last && !calendar.last.ok && "text-destructive")}>{calendarText}</span>
          </button>
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
          size="sm"
          disabled={pending.length === 0 || exporting}
          title={config.sheetUrl ? (config.sheetLink ?? "Google Sheets") : (config.filePath ?? "")}
          onClick={run(async () => {
            setExporting(true);
            try {
              setNotice(exportNotice(await api.exportTimesheet(start, rangeDays)));
            } finally {
              setExporting(false);
            }
          })}
        >
          {config.sheetUrl ? <Sheet /> : <FileSpreadsheet />}
          {config.sheetUrl ? "Sheets'e aktar" : "Excel'e aktar"}
          {pending.length ? ` (${pending.length})` : ""}
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
      {notice && (
        <div className="flex items-start gap-2 rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-xs">
          <Check className="mt-0.5 size-3.5 shrink-0 text-success" />
          <span className="min-w-0 flex-1 break-words selectable">{notice}</span>
          <button aria-label="Kapat" onClick={() => setNotice(null)}>
            <X className="size-3.5 text-muted-foreground" />
          </button>
        </div>
      )}

      <datalist id="timesheet-details">
        {details.slice(0, 200).map((d) => (
          <option key={d} value={d} />
        ))}
      </datalist>
      <datalist id="timesheet-parties">
        {[...new Set([config.defaultParty, config.company, ...config.projects.map((p) => p.party ?? "")])]
          .filter(Boolean)
          .map((p) => (
            <option key={p} value={p} />
          ))}
      </datalist>

      {days.map((d) => (
        <DayCard
          key={d.date}
          day={d}
          config={config}
          projects={projects}
          onOpenDay={onOpenDay}
          run={run}
          // Gün görünümünde boş gün de gösterilir (yoksa sayfa boş kalır).
          alwaysShow={mode === "day"}
        />
      ))}
      {mode !== "day" && days.length > 0 && days.every((d) => d.entries.length === 0) && (
        <p className="px-1 text-sm text-muted-foreground">Bu dönemde kayıt yok.</p>
      )}
    </div>
  );
}

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

function DayCard({
  day,
  config,
  projects,
  onOpenDay,
  run,
  alwaysShow,
}: {
  day: TimesheetDay;
  config: TimesheetConfig;
  projects: Tag[];
  onOpenDay: (iso: string) => void;
  run: Run;
  alwaysShow: boolean;
}) {
  const [confirmReset, setConfirmReset] = useState(false);
  const date = parseIsoDate(day.date);
  const total = day.entries.reduce((s, e) => s + e.hours, 0);
  const totalActual = day.entries.reduce((s, e) => s + worked(e), 0);
  const exported = day.entries.length > 0 && day.entries.every((e) => e.exported);
  const empty = day.entries.length === 0 && day.unassignedSeconds < 60 && day.meetings.length === 0;
  const weekend = date.getDay() === 0 || date.getDay() === 6;
  if (empty && weekend && !alwaysShow) return null;

  const firstMapping = config.projects[0];
  const blank: TimesheetEntry = {
    date: day.date,
    start: "09:00:00",
    hours: 1,
    kind: "Working",
    details: "",
    party: config.defaultParty,
    projectId: firstMapping?.projectId ?? projects[0]?.id ?? "",
    division: firstMapping?.division ?? projects[0]?.name ?? "",
  };

  return (
    <section className="rounded-xl border bg-card shadow-xs">
      <div className="flex flex-wrap items-center gap-2 px-4 py-2.5">
        <span className="text-[13px] font-semibold capitalize">{dayFmt.format(date)}</span>
        <span className="text-xs text-muted-foreground tabular">
          {total ? manDays(total, config.dayHours) : "—"}
          {total > 0 && <span title="Takip edilen gerçek süre"> · gerçek {actual(totalActual)}</span>}
        </span>
        {day.entries.length > 0 && (
          <Badge variant="outline" className={cn(exported && "border-success/40 text-success")}>
            {exported ? "Aktarıldı" : day.approved ? "Onaylandı" : "Öneri"}
          </Badge>
        )}
        {day.unassignedSeconds >= 60 && (
          <button
            className="text-xs text-muted-foreground underline-offset-2 hover:underline"
            onClick={() => onOpenDay(day.date)}
            title="Takvimde aç: blokları ya da aralıkları projeye ata"
          >
            Projesiz {formatDuration(day.unassignedSeconds)} · takvimde ata
          </button>
        )}
        <span className="ml-auto flex gap-1.5">
          {!day.approved && day.entries.length > 0 && (
            <Button size="sm" onClick={run(() => api.approveTimesheetDay(day.date))}>
              <Check /> Onayla
            </Button>
          )}
          {day.approved && exported && (
            // Aktarılmış satırlar korunur ve tekrar önerilmez: aktarımdan sonra yapılan iş eklenir.
            <Button
              size="sm"
              variant="ghost"
              onClick={run(() => api.approveTimesheetDay(day.date))}
              title="Aktarımdan sonra takip edilen işi öner; aktarılan satırlar değişmez"
            >
              <RefreshCw /> Yeni işleri öner
            </Button>
          )}
          {day.approved &&
            !exported &&
            (confirmReset ? (
              <Button
                size="sm"
                variant="destructive"
                onClick={() => {
                  // Onay tek kullanımlık: sonraki tıklama yeniden onay istesin.
                  setConfirmReset(false);
                  run(() => api.approveTimesheetDay(day.date))();
                }}
              >
                Düzenlemeler silinsin, yeniden öner
              </Button>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setConfirmReset(true)}
                title="Satırları takip verisinden yeniden üret"
              >
                <RefreshCw /> Yeniden öner
              </Button>
            ))}
          {(day.approved || day.entries.length === 0) && blank.projectId && (
            <Button size="sm" variant="outline" onClick={run(() => api.saveTimesheetEntry(null, blank))}>
              <Plus /> Satır
            </Button>
          )}
        </span>
      </div>
      {day.entries.length > 0 && (
        // Dar pencerede açıklama sütunu ezilmesin: satırlar kart içinde yatay kayar.
        <div className="overflow-x-auto border-t">
          <div className="grid grid-cols-[112px_128px_96px_minmax(160px,1fr)_96px_minmax(110px,180px)_28px] gap-2 px-4 pt-2 text-[11px] text-muted-foreground">
            <span>Başlangıç</span>
            <span>Saat</span>
            <span>Tür</span>
            <span>Açıklama</span>
            <span>Taraf</span>
            <span>Birim</span>
            <span />
          </div>
          <ul className="pb-1.5">
            {day.entries.map((e, i) => (
              <EntryRow
                key={e.id ?? `oneri-${i}`}
                entry={e}
                editable={day.approved && !e.exported}
                config={config}
                projects={projects}
                run={run}
              />
            ))}
          </ul>
          {!day.approved && (
            <p className="px-4 pb-2.5 text-[11px] text-muted-foreground">
              Bunlar takip verisinden öneriler; düzenlemek ve aktarmak için günü onayla.
            </p>
          )}
        </div>
      )}
      {day.meetings.length > 0 && <MeetingList date={day.date} meetings={day.meetings} projects={projects} run={run} />}
    </section>
  );
}

/**
 * Takvimde olup hiçbir projeye düşmeyen toplantılar. Seçilen proje serinin tüm tekrarlarına
 * uygulanır (haftalık toplantı bir kez atanır); "Yoksay" seriyi zaman çizelgesinden çıkarır.
 */
function MeetingList({
  date,
  meetings,
  projects,
  run,
}: {
  date: string;
  meetings: Meeting[];
  projects: Tag[];
  run: Run;
}) {
  return (
    <div className="border-t px-4 py-2">
      <div className="flex items-center gap-1.5 pb-1 text-[11px] text-muted-foreground">
        <CalendarDays className="size-3.5" /> Takvimden, projesi belli olmayan toplantılar
      </div>
      <ul className="space-y-1">
        {meetings.map((m) => (
          <li key={`${m.uid}-${m.start}`} className="flex flex-wrap items-center gap-2 text-xs">
            <span className="w-24 shrink-0 text-muted-foreground tabular">
              {timeFmt.format(new Date(m.start))}–{timeFmt.format(new Date(m.end))}
            </span>
            <span className="min-w-0 flex-1 truncate" title={m.location || undefined}>
              {m.subject || "(konusuz)"}
              <span className="ml-1.5 text-muted-foreground">{m.online ? "Online" : "F2F"}</span>
            </span>
            <Select
              value=""
              onValueChange={(v) => run(() => api.assignMeeting(m.uid, v === IGNORE ? null : v, date))()}
            >
              <SelectTrigger size="sm" className="h-7 w-44 text-xs" aria-label={`${m.subject} projesi`}>
                <SelectValue placeholder="Projeye ata…" />
              </SelectTrigger>
              <SelectContent>
                {projects.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
                    {p.name}
                  </SelectItem>
                ))}
                <SelectItem value={IGNORE}>Yoksay (zaman çizelgesine alma)</SelectItem>
              </SelectContent>
            </Select>
          </li>
        ))}
      </ul>
    </div>
  );
}

function EntryRow({
  entry,
  editable,
  config,
  projects,
  run,
}: {
  entry: EntryView;
  editable: boolean;
  config: TimesheetConfig;
  projects: Tag[];
  run: Run;
}) {
  const [draft, setDraft] = useState(entry);
  // Her yeniden yüklemede satırlar yeni nesne olarak gelir; yalnızca içerik değişince taslak
  // sıfırlansın (başka satırın kaydı ya da takvim yenilemesi yazılanı silmesin).
  const entryJson = JSON.stringify(entry);
  useEffect(() => setDraft(JSON.parse(entryJson)), [entryJson]);
  const save = (next: TimesheetEntry) => run(() => api.saveTimesheetEntry(entry.id, next))();
  const commit = () => {
    if (JSON.stringify(draft) !== JSON.stringify(entry)) save(draft);
  };
  const divisions = useMemo(() => {
    const list = config.projects.map((m) => ({ projectId: m.projectId, division: m.division }));
    for (const p of projects)
      if (!list.some((m) => m.projectId === p.id)) list.push({ projectId: p.id, division: p.name });
    return list;
  }, [config.projects, projects]);
  const color = tagColor(projects.find((p) => p.id === draft.projectId));
  const cell = "h-7 px-1.5 text-xs";

  return (
    <li
      className={cn(
        "grid grid-cols-[112px_128px_96px_minmax(160px,1fr)_96px_minmax(110px,180px)_28px] items-center gap-2 px-4 py-1",
        !editable && "text-muted-foreground",
      )}
    >
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
        className={cell}
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
      <Select
        value={draft.projectId}
        disabled={!editable}
        onValueChange={(id) => {
          const d = divisions.find((x) => x.projectId === id);
          if (d) save({ ...draft, projectId: id, division: d.division });
        }}
      >
        <SelectTrigger size="sm" className="h-7 min-w-0 text-xs" aria-label="Birim">
          <i className="size-2 shrink-0 rounded-full" style={{ background: color }} />
          <SelectValue placeholder={draft.division} />
        </SelectTrigger>
        <SelectContent>
          {divisions.map((d) => (
            <SelectItem key={d.projectId} value={d.projectId}>
              {d.division}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {entry.exported ? (
        <Check className="size-3.5 text-success" aria-label="Excel'e aktarıldı" />
      ) : editable && entry.id ? (
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-7 text-muted-foreground hover:text-destructive"
          aria-label="Satırı sil"
          onClick={run(() => api.deleteTimesheetEntry(entry.id!))}
        >
          <Trash2 />
        </Button>
      ) : (
        <span />
      )}
    </li>
  );
}

/**
 * İlk kurulum: kayıtların yazılacağı dosya (Excel ya da Google Sheets) ve isteğe bağlı Outlook
 * takvimi. Sonradan Ayarlar → Bağlantılar'dan değiştirilir.
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
          Projeye atanmış çalışma süren ve takvimindeki toplantılar günlük iş kayıtlarına dönüşür; onayladığın kayıtlar
          firmanın dosyasına aynı sütun ve biçimle eklenir.
        </p>
      </div>
      <section className={step}>
        <h2 className="text-[13px] font-semibold">1. Kayıtlar nereye yazılsın?</h2>
        <p className="text-xs text-muted-foreground">
          Firma, danışman, birimler (projeler) ve geçmiş açıklamalar seçtiğin dosyadan alınır.
        </p>
        {sheets ? (
          <SheetConnect onDone={onDone} onCancel={() => setSheets(false)} />
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
                    await api.importTimesheetTemplate(path);
                    onDone();
                  }
                } catch (e) {
                  setError(String(e));
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
        İkisini de sonra <b>Ayarlar → Bağlantılar</b>'dan değiştirebilirsin.
      </p>
    </div>
  );
}
