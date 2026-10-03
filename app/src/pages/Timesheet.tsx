import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight, FileSpreadsheet, Plus, RefreshCw, Settings2, Trash2, X } from "lucide-react";
import {
  api,
  formatDuration,
  type EntryKind,
  type EntryView,
  type Tag,
  type TimesheetConfig,
  type TimesheetDay,
  type TimesheetEntry,
} from "../api";
import { ErrorText } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { addDays, formatWeek, isoDate, parseIsoDate, startOfWeek, today } from "../lib/dates";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

const KINDS: EntryKind[] = ["Working", "Online", "F2F"];
const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric", month: "short" });
const num = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** Saat, adam-gün (saat / günlük saat; yuvarlanmaz). */
function manDays(hours: number, dayHours: number) {
  return `${num.format(hours)} sa · ${num.format(hours / (dayHours || 8))} ag`;
}

/**
 * Zaman çizelgesi: haftanın günleri için projeye atanmış süreden önerilen iş kayıtları.
 * Gün onaylanınca satırlar düzenlenebilir; onaylı ve aktarılmamış satırlar Excel dosyasına eklenir.
 */
export default function Timesheet({ onOpenDay }: { onOpenDay: (iso: string) => void }) {
  const [week, setWeek] = useState(() => isoDate(startOfWeek(today())));
  const [config, setConfig] = useState<TimesheetConfig | null>(null);
  const [days, setDays] = useState<TimesheetDay[]>([]);
  const [details, setDetails] = useState<string[]>([]);
  const [projects, setProjects] = useState<Tag[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [settings, setSettings] = useState(false);

  const load = useCallback(async () => {
    try {
      const [c, d, det, tax] = await Promise.all([
        api.timesheetConfig(),
        api.timesheetDays(week, 7),
        api.timesheetDetails(),
        api.taxonomy(),
      ]);
      setConfig(c);
      setDays(d);
      setDetails(det);
      setProjects(tax.tags.filter((t) => t.kind === "project"));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [week]);
  useEffect(() => {
    load();
  }, [load]);

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
  if (!config.filePath) return <Setup onDone={load} />;

  const all = days.flatMap((d) => d.entries);
  const pending = days.filter((d) => d.approved).flatMap((d) => d.entries.filter((e) => !e.exported));
  const total = all.reduce((s, e) => s + e.hours, 0);
  const byDivision = new Map<string, number>();
  for (const e of all) byDivision.set(e.division, (byDivision.get(e.division) ?? 0) + e.hours);
  const thisWeek = isoDate(startOfWeek(today()));

  return (
    <div className="mx-auto w-full max-w-5xl space-y-4 px-6 pt-2 pb-10">
      <div className="flex flex-wrap items-center gap-2">
        <div className="mr-auto">
          <h1 className="text-[15px] font-semibold">
            {config.company || "Zaman çizelgesi"} · {formatWeek(parseIsoDate(week))}
          </h1>
          <p className="text-xs text-muted-foreground">
            Toplam {manDays(total, config.dayHours)}
            {[...byDivision.entries()].map(([d, h]) => ` · ${d}: ${num.format(h)} sa`).join("")}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Önceki hafta"
          onClick={() => setWeek(isoDate(addDays(parseIsoDate(week), -7)))}
        >
          <ChevronLeft />
        </Button>
        <Button variant="outline" size="sm" disabled={week === thisWeek} onClick={() => setWeek(thisWeek)}>
          Bu hafta
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Sonraki hafta"
          disabled={week >= thisWeek}
          onClick={() => setWeek(isoDate(addDays(parseIsoDate(week), 7)))}
        >
          <ChevronRight />
        </Button>
        <Button variant="outline" size="sm" onClick={() => setSettings((s) => !s)} aria-expanded={settings}>
          <Settings2 /> Ayarlar
        </Button>
        <Button
          size="sm"
          disabled={pending.length === 0}
          title={config.filePath}
          onClick={run(async () => {
            const r = await api.exportTimesheet(week, 7);
            setNotice(
              `${r.rows} satır Excel'e eklendi (${r.filled} boş satıra, ${r.inserted} yeni satır). Yedek: ${r.backup}`,
            );
          })}
        >
          <FileSpreadsheet /> Excel'e aktar{pending.length ? ` (${pending.length})` : ""}
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
      {settings && <SettingsPanel config={config} projects={projects} onSaved={load} />}

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
        <DayCard key={d.date} day={d} config={config} projects={projects} onOpenDay={onOpenDay} run={run} />
      ))}
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
}: {
  day: TimesheetDay;
  config: TimesheetConfig;
  projects: Tag[];
  onOpenDay: (iso: string) => void;
  run: Run;
}) {
  const [confirmReset, setConfirmReset] = useState(false);
  const date = parseIsoDate(day.date);
  const total = day.entries.reduce((s, e) => s + e.hours, 0);
  const exported = day.entries.length > 0 && day.entries.every((e) => e.exported);
  const empty = day.entries.length === 0 && day.unassignedSeconds < 60;
  const weekend = date.getDay() === 0 || date.getDay() === 6;
  if (empty && weekend) return null;

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
        <span className="text-xs text-muted-foreground tabular">{total ? manDays(total, config.dayHours) : "—"}</span>
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
          {day.approved &&
            !exported &&
            (confirmReset ? (
              <Button size="sm" variant="destructive" onClick={run(() => api.approveTimesheetDay(day.date))}>
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
        <div className="border-t">
          <div className="grid grid-cols-[92px_70px_96px_minmax(0,1fr)_96px_minmax(0,180px)_28px] gap-2 px-4 pt-2 text-[11px] text-muted-foreground">
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
              Bunlar takip verisinden öneriler; düzenlemek ve Excel'e aktarmak için günü onayla.
            </p>
          )}
        </div>
      )}
    </section>
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
  useEffect(() => setDraft(entry), [entry]);
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
        "grid grid-cols-[92px_70px_96px_minmax(0,1fr)_96px_minmax(0,180px)_28px] items-center gap-2 px-4 py-1",
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
      <Input
        type="number"
        step="0.05"
        min="0.01"
        className={cn(cell, "tabular")}
        disabled={!editable}
        value={Number(draft.hours.toFixed(2))}
        onChange={(e) => setDraft({ ...draft, hours: Number(e.target.value) })}
        onBlur={commit}
        aria-label="Saat"
      />
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

/** İlk kurulum: firmanın Excel şablonunu seç ve içe aktar. */
function Setup({ onDone }: { onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <div className="mx-auto w-full max-w-xl space-y-4 px-6 pt-10">
      <div className="space-y-3 rounded-xl border bg-card px-6 py-5 shadow-xs">
        <FileSpreadsheet className="size-6 text-primary" />
        <h1 className="text-base font-semibold">Zaman çizelgesi</h1>
        <p className="text-sm text-muted-foreground">
          Projeye atanmış çalışma süren günlük iş kayıtlarına dönüşür; onayladığın kayıtlar firmanın Excel dosyasına
          (aynı sütun ve biçimle) eklenir. Başlamak için o dosyayı seç: firma, danışman, taraflar, birimler (projeler)
          ve geçmiş açıklamalar dosyadan alınır.
        </p>
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
          <FileSpreadsheet /> Excel dosyasını seç
        </Button>
        <ErrorText>{error}</ErrorText>
        <p className="text-xs text-muted-foreground">
          Dosyaya yazmadan önce her seferinde yanına zaman damgalı bir yedek alınır.
        </p>
      </div>
    </div>
  );
}

function SettingsPanel({
  config,
  projects,
  onSaved,
}: {
  config: TimesheetConfig;
  projects: Tag[];
  onSaved: () => void;
}) {
  const [c, setC] = useState(config);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setC(config), [config]);
  const save = async (next: TimesheetConfig) => {
    setC(next);
    try {
      await api.saveTimesheetConfig(next);
      onSaved();
    } catch (e) {
      setError(String(e));
    }
  };
  const mapping = (id: string) => c.projects.find((m) => m.projectId === id);
  const setMapping = (id: string, patch: { division?: string; party?: string | null }) => {
    const current = mapping(id) ?? {
      projectId: id,
      division: projects.find((p) => p.id === id)?.name ?? "",
      party: null,
    };
    const next = { ...current, ...patch };
    save({ ...c, projects: [...c.projects.filter((m) => m.projectId !== id), next] });
  };
  const field = (label: string, value: string, onCommit: (v: string) => void, hint?: string) => (
    <label className="space-y-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <Input
        className="h-8 text-sm"
        defaultValue={value}
        onBlur={(e) => e.target.value !== value && onCommit(e.target.value.trim())}
        title={hint}
      />
    </label>
  );

  return (
    <section className="space-y-4 rounded-xl border bg-card px-4 py-4 shadow-xs">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        {field("Firma", c.company, (v) => save({ ...c, company: v }))}
        {field("Danışman (Consultant)", c.consultant, (v) => save({ ...c, consultant: v }))}
        {field("Varsayılan taraf (Parties)", c.defaultParty, (v) => save({ ...c, defaultParty: v }))}
        {field("Adam-gün saati", String(c.dayHours), (v) => {
          const n = Number(v.replace(",", "."));
          if (n > 0 && n <= 24) save({ ...c, dayHours: n });
        })}
      </div>
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="text-muted-foreground">Excel dosyası:</span>
        <span className="min-w-0 flex-1 truncate font-medium selectable" title={c.filePath ?? ""}>
          {c.filePath}
        </span>
        <Button
          size="sm"
          variant="outline"
          onClick={async () => {
            try {
              const path = await api.pickTimesheetFile();
              if (path) {
                await api.importTimesheetTemplate(path);
                onSaved();
              }
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          Değiştir / yeniden içe aktar
        </Button>
      </div>
      <div className="space-y-1.5">
        <div className="text-xs text-muted-foreground">Proje → birim (Excel'deki ad) ve taraf</div>
        <ul className="divide-y rounded-lg border">
          {projects.map((p) => (
            <li
              key={p.id}
              className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_120px] items-center gap-2 px-3 py-1.5"
            >
              <span className="flex min-w-0 items-center gap-2 text-[13px]">
                <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
                <span className="truncate">{p.name}</span>
              </span>
              <Input
                className="h-7 text-xs"
                defaultValue={mapping(p.id)?.division ?? p.name}
                onBlur={(e) => setMapping(p.id, { division: e.target.value.trim() })}
                aria-label={`${p.name} birimi`}
              />
              <Input
                className="h-7 text-xs"
                placeholder={c.defaultParty}
                defaultValue={mapping(p.id)?.party ?? ""}
                onBlur={(e) => setMapping(p.id, { party: e.target.value.trim() || null })}
                aria-label={`${p.name} tarafı`}
              />
            </li>
          ))}
        </ul>
      </div>
      <ErrorText>{error}</ErrorText>
    </section>
  );
}
