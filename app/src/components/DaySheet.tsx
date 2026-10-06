import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Check, Loader2, Send } from "lucide-react";
import {
  api,
  type EntryKind,
  type EntryView,
  formatDuration,
  type Report,
  type Tag,
  type TimesheetDay,
  type UnassignedMeeting,
} from "../api";
import { HATCH } from "./Calendar";
import { ProjectSelect } from "./ProjectSelect";
import Toolbar from "./Toolbar";
import { Button } from "./ui/button";
import { isoDate, parseIsoDate, today } from "../lib/dates";
import { friendlyError, toast, undoable, useChanged } from "../lib/feedback";
import { tagColor, tagInk, UNASSIGNED } from "../lib/tags";
import { blocked, isStale, needsDetails, started } from "../lib/timesheet";
import { useTauriEvent } from "../lib/useTauriEvent";
import { cn } from "../lib/utils";
import { exportNotice, KINDS, num, toRef } from "../pages/timesheet/shared";

/** Şeridin yüksekliği tabloya uyar: satır başına yaklaşık bu kadar (piksel), en az `STRIP_MIN`. */
const ROW_PX = 39;
const STRIP_MIN = 320;
/** Düz metin hücreleri düzenlenebilir alanların yazısıyla aynı hizada başlasın (kenar + iç boşluk). */
const PLAIN = "pl-[15px]";
const hm = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });

type Props = {
  day: string;
  title: string;
  report: Report;
  tags: Map<string, Tag>;
  projects: Tag[];
  /** Üst çubuğun sağındaki görünüm seçici ve gezinti. */
  controls: ReactNode;
  /** Atama ya da satır değişince takvim de yenilensin. */
  onChanged: () => void;
};

/** Tablodaki bir satır: çizelge satırı, projesiz blok, çizelgeye gitmeyen proje ya da toplantı. */
type Line =
  | { kind: "entry"; key: string; from: number; to: number; sheetId: string; entry: EntryView }
  | { kind: "unassigned"; key: string; from: number; to: number; app: string }
  | { kind: "off"; key: string; from: number; to: number; projectId: string; app: string }
  | { kind: "meeting"; key: string; from: number; to: number; meeting: UnassignedMeeting };

/**
 * Gün raporunun Çizelge görünümü: solda günün blokları dikey şeritte, sağda her bloğun karşılığı
 * olan zaman çizelgesi satırı. Satırlar yerinde düzenlenir, projesiz bloklar burada atanır, gün
 * tek düğmeyle gönderilir. Üzerine gelinen blok ile satır birbirini vurgular.
 */
export default function DaySheet({ day, title, report, tags, projects, controls, onChanged }: Props) {
  const [sheets, setSheets] = useState<{ id: string; projects: Set<string>; day: TimesheetDay | null }[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [hover, setHover] = useState<[number, number] | null>(null);

  const seq = useRef(0);
  const load = useCallback(async () => {
    const n = ++seq.current;
    try {
      const config = await api.timesheetConfig();
      const list = config.timesheets.filter((t) => t.projects.length > 0);
      const days = await Promise.all(list.map((t) => api.timesheetDays(t.id, day, 1)));
      if (n !== seq.current) return;
      setSheets(
        list.map((t, i) => ({
          id: t.id,
          projects: new Set(t.projects.map((m) => m.projectId)),
          day: days[i][0] ?? null,
        })),
      );
      setError(null);
    } catch (e) {
      if (n === seq.current) setError(friendlyError(e));
    }
  }, [day]);
  useEffect(() => {
    load();
  }, [load, report]);
  useChanged(load);
  useTauriEvent(api.onSync, load);

  // Süren kayıtlar (alandan çıkınca kaydedilen düzenleme): gönderim onları bekler.
  const inFlight = useRef(new Set<Promise<unknown>>());
  const run = async (work: () => Promise<unknown>) => {
    setError(null);
    const p = work();
    inFlight.current.add(p);
    try {
      await p;
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      inFlight.current.delete(p);
      await load();
      onChanged();
    }
  };

  const dayStart = +parseIsoDate(day);
  const now = Date.now();
  const lines = useMemo<Line[]>(() => {
    const out: Line[] = [];
    const sheetProjects = new Set<string>();
    for (const s of sheets ?? []) {
      for (const id of s.projects) sheetProjects.add(id);
      for (const e of s.day?.entries ?? []) {
        const [h, m, sec] = e.start.split(":").map(Number);
        const from = +parseIsoDate(e.date) + ((h * 60 + m) * 60 + (sec || 0)) * 1000;
        out.push({ kind: "entry", key: `e-${e.key}`, from, to: from + e.hours * 3600_000, sheetId: s.id, entry: e });
      }
      for (const m of s.day?.meetings ?? []) {
        out.push({
          kind: "meeting",
          key: `m-${m.uid}-${m.start}`,
          from: +new Date(m.start),
          to: +new Date(m.end),
          meeting: m,
        });
      }
    }
    for (const b of report.work.blocks) {
      const from = +new Date(b.start);
      const to = +new Date(b.end);
      const app = b.topApps[0]?.appName ?? "";
      if (!b.projectId) out.push({ kind: "unassigned", key: `u-${b.start}`, from, to, app });
      else if (sheets && !sheetProjects.has(b.projectId))
        out.push({ kind: "off", key: `o-${b.start}`, from, to, projectId: b.projectId, app });
    }
    return out.sort((a, b) => a.from - b.from);
  }, [sheets, report]);

  const entries = lines.flatMap((l) => (l.kind === "entry" ? [l] : []));
  const nowDate = new Date(now);
  // Süren iş (bitişi şimdiden sonra) henüz gönderilmez.
  const running = (l: Line) => l.from <= now && l.to > now;
  const ready = entries.filter(
    (l) => !l.entry.exported && started(l.entry, nowDate) && !running(l) && !blocked(l.entry),
  );
  const missing =
    entries.filter((l) => blocked(l.entry)).length +
    lines.filter((l) => l.kind === "unassigned" || l.kind === "meeting").length;
  const allSent = entries.length > 0 && entries.every((l) => l.entry.exported);

  async function send() {
    if (ready.length === 0) return;
    setSending(true);
    setError(null);
    try {
      // Düğmeye basınca alandan çıkılır ve düzenleme kaydedilir: yazılan hali gitsin.
      await Promise.allSettled([...inFlight.current]);
      const bySheet = new Map<string, EntryView[]>();
      for (const l of ready) bySheet.set(l.sheetId, [...(bySheet.get(l.sheetId) ?? []), l.entry]);
      for (const [id, rows] of bySheet) {
        const r = await api.exportTimesheet(id, rows.map(toRef));
        toast(exportNotice(r), {
          tone: "success",
          action: {
            label: "Geri al",
            run: () =>
              api.undoLastExport().then(
                (m) => {
                  toast(m, { tone: "success" });
                  void load();
                  onChanged();
                },
                (e) => toast(friendlyError(e), { tone: "error" }),
              ),
          },
        });
      }
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setSending(false);
      await load();
      onChanged();
    }
  }

  const overlaps = (l: { from: number; to: number }) => !!hover && l.from < hover[1] && l.to > hover[0];
  const stats =
    entries.length === 0
      ? formatDuration(report.totalSeconds)
      : allSent
        ? `${formatDuration(report.totalSeconds)} · gönderildi`
        : `${formatDuration(report.totalSeconds)} · ${ready.length} satır hazır${missing ? `, ${missing} eksik` : ""}`;

  return (
    <>
      <Toolbar title={title}>
        <span className="hidden text-xs whitespace-nowrap text-muted-foreground tabular xl:inline">{stats}</span>
        <Button
          size="sm"
          disabled={sending || ready.length === 0}
          onClick={send}
          title={
            ready.length === 0
              ? "Gönderilecek hazır satır yok"
              : `Hazır ${ready.length} satırı gönder${missing ? ` (${missing} eksik satır gönderilmez)` : ""}`
          }
        >
          {sending ? <Loader2 className="animate-spin" /> : <Send />}
          Günü kapat ve gönder
        </Button>
        {controls}
      </Toolbar>
      <div className="@container flex-1 overflow-y-auto px-5 pb-6">
        {error && <p className="pb-3 text-xs text-destructive selectable">{error}</p>}
        <div className="grid items-start gap-4 @[640px]:grid-cols-[150px_minmax(0,1fr)]">
          <Strip
            report={report}
            tags={tags}
            dayStart={dayStart}
            now={now}
            hover={hover}
            onHover={setHover}
            height={Math.max(STRIP_MIN, 36 + lines.length * ROW_PX)}
          />
          <div className="min-w-0 overflow-x-auto">
            {!sheets ? (
              <div className="skeleton h-40 rounded-xl" aria-busy />
            ) : lines.length === 0 ? (
              <p className="py-6 text-center text-xs text-muted-foreground">Bu gün için satır yok.</p>
            ) : (
              <table className="w-full min-w-[620px] border-collapse text-xs">
                <thead>
                  <tr className="text-left text-[11px] text-muted-foreground">
                    {["Başlangıç", "Saat", "Tür", "Proje", "Açıklama"].map((h) => (
                      <th key={h} className="border-b px-2 py-1.5 font-medium">
                        {h}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="tabular">
                  {lines.map((l) => (
                    <Row
                      key={l.key}
                      line={l}
                      tags={tags}
                      projects={projects}
                      running={running(l)}
                      lit={overlaps(l)}
                      onHover={(on) => setHover(on ? [l.from, l.to] : null)}
                      run={run}
                    />
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

function Row({
  line: l,
  tags,
  projects,
  running,
  lit,
  onHover,
  run,
}: {
  line: Line;
  tags: Map<string, Tag>;
  projects: Tag[];
  running: boolean;
  lit: boolean;
  onHover: (on: boolean) => void;
  run: (work: () => Promise<unknown>) => Promise<void>;
}) {
  const td = "border-b px-2 py-1.5 align-middle";
  const start = hm.format(new Date(l.from));
  const hours = num.format((l.to - l.from) / 3600_000);
  const rowProps = {
    onMouseEnter: () => onHover(true),
    onMouseLeave: () => onHover(false),
  };
  if (l.kind === "entry") return <EntryLine {...rowProps} line={l} tags={tags} running={running} lit={lit} run={run} />;
  if (l.kind === "off") {
    const tag = tags.get(l.projectId);
    return (
      <tr {...rowProps} className={cn("text-muted-foreground/70", lit && "bg-accent")}>
        <td className={td}>{start}</td>
        <td className={cn(td, PLAIN)}>{hours}</td>
        <td className={td}>
          <Chip kind="Working" />
        </td>
        <td className={td}>
          <ProjectName tag={tag} />
        </td>
        <td className={cn(td, PLAIN)}>{tag ? "Zaman çizelgesine bağlı değil · gönderilmez" : l.app}</td>
      </tr>
    );
  }
  // Projesiz blok ya da projesi belli olmayan toplantı: proje seçilince satır olur.
  const assign = (id: string) =>
    l.kind === "meeting"
      ? run(() => api.assignMeeting(l.meeting.uid, id))
      : run(() =>
          undoable(
            api.setRangeProject(new Date(l.from).toISOString(), new Date(l.to).toISOString(), id),
            `${tags.get(id)?.name ?? "Projeye"} atandı`,
          ),
        );
  const suggestion = l.kind === "meeting" ? l.meeting.suggestion : null;
  const suggested = suggestion ? tags.get(suggestion.projectId) : undefined;
  return (
    <tr {...rowProps} className={cn("bg-amber-500/10", lit && "bg-amber-500/20")}>
      <td className={td}>{start}</td>
      <td className={cn(td, PLAIN)}>{hours}</td>
      <td className={td}>
        {l.kind === "meeting" ? <Chip kind={l.meeting.online ? "Online" : "F2F"} /> : <Chip kind={null} />}
      </td>
      <td className={td}>
        <div className="flex items-center gap-1.5">
          <ProjectSelect
            value=""
            projects={projects}
            placeholder="Proje seç"
            className="h-7 w-36 border-amber-500/40 text-xs font-medium text-amber-700 dark:text-amber-400"
            aria-label={`${start} için proje`}
            onChange={assign}
          />
          {suggestion && suggested && (
            <button
              className="truncate rounded-md border border-dashed px-1.5 py-0.5 text-[11px] text-muted-foreground hover:border-solid hover:text-foreground"
              title={`Öneri: ${suggestion.reason}`}
              onClick={() => assign(suggestion.projectId)}
            >
              → {suggested.name}
            </button>
          )}
        </div>
      </td>
      <td className={cn(td, PLAIN, "text-muted-foreground")}>
        {l.kind === "meeting" ? l.meeting.subject || "(konusuz)" : l.app || UNASSIGNED}
      </td>
    </tr>
  );
}

/** Çizelge satırı: saat, tür ve açıklama yerinde düzenlenir (gönderilmişse salt okunur). */
function EntryLine({
  line,
  tags,
  running,
  lit,
  run,
  onMouseEnter,
  onMouseLeave,
}: {
  line: Extract<Line, { kind: "entry" }>;
  tags: Map<string, Tag>;
  running: boolean;
  lit: boolean;
  run: (work: () => Promise<unknown>) => Promise<void>;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}) {
  const e = line.entry;
  const [hours, setHours] = useState(num.format(e.hours));
  const [details, setDetails] = useState(e.details);
  useEffect(() => setHours(num.format(e.hours)), [e.hours]);
  useEffect(() => setDetails(e.details), [e.details]);
  const save = (patch: Partial<EntryView>) => {
    const next = { ...e, ...patch };
    if ((Object.keys(patch) as (keyof EntryView)[]).every((k) => next[k] === e[k])) return;
    void run(() => api.saveTimesheetEntry(e.id, next));
  };
  const commitHours = () => {
    const h = Number(hours.replace(",", "."));
    if (!(h > 0)) return setHours(num.format(e.hours));
    save({ hours: Math.round(h * 4) / 4 });
  };
  const td = "border-b px-2 py-1 align-middle";
  const field =
    "h-7 w-full rounded-md border border-transparent bg-transparent px-1.5 outline-none hover:border-input focus:border-input focus:bg-background focus-visible:ring-2 focus-visible:ring-ring/40";
  const tag = tags.get(e.projectId);
  if (e.exported)
    return (
      <tr onMouseEnter={onMouseEnter} onMouseLeave={onMouseLeave} className={cn(lit && "bg-accent")}>
        <td className={cn(td, "py-1.5")}>{e.start.slice(0, 5)}</td>
        <td className={cn(td, PLAIN, "py-1.5")}>{num.format(e.hours)}</td>
        <td className={td}>
          <Chip kind={e.kind} />
        </td>
        <td className={td}>
          <ProjectName tag={tag} />
        </td>
        <td className={cn(td, PLAIN, "text-muted-foreground")}>
          <span className="flex items-center gap-1.5">
            <span className="min-w-0 flex-1 truncate">{e.details}</span>
            <Check className="size-3.5 shrink-0 text-success" aria-label="Gönderildi" />
          </span>
        </td>
      </tr>
    );
  const empty = needsDetails({ ...e, details });
  return (
    <tr
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      className={cn(isStale(e) && "bg-amber-500/5", lit && "bg-accent")}
    >
      <td className={cn(td, "w-16 py-1.5")}>{e.start.slice(0, 5)}</td>
      <td className={cn(td, "w-20")}>
        {running ? (
          <span className="pl-[7px] text-muted-foreground">sürüyor</span>
        ) : (
          <input
            className={cn(field, "w-14")}
            value={hours}
            inputMode="decimal"
            onChange={(ev) => setHours(ev.target.value)}
            onBlur={commitHours}
            onKeyDown={(ev) => ev.key === "Enter" && ev.currentTarget.blur()}
            aria-label="Saat"
          />
        )}
      </td>
      <td className={cn(td, "w-24")}>
        <select
          className={cn(
            chipClass(e.kind),
            "cursor-pointer appearance-none outline-none focus-visible:ring-2 focus-visible:ring-ring/40",
          )}
          value={e.kind}
          onChange={(ev) => save({ kind: ev.target.value as EntryKind })}
          aria-label="Tür"
        >
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </td>
      <td className={cn(td, "w-40")}>
        <ProjectName tag={tag} />
      </td>
      <td className={td}>
        <input
          className={cn(field, empty && "border-amber-500/40 bg-amber-500/5")}
          value={details}
          placeholder={empty ? "Açıklama yaz" : undefined}
          onChange={(ev) => setDetails(ev.target.value)}
          onBlur={() => save({ details: details.trim() })}
          onKeyDown={(ev) => ev.key === "Enter" && ev.currentTarget.blur()}
          aria-label="Açıklama"
          title={isStale(e) ? "Takipte değişti: işin bir kısmı başka projeye alınmış" : undefined}
        />
      </td>
    </tr>
  );
}

function chipClass(kind: EntryKind | null) {
  return cn(
    "rounded-full px-2 py-0.5 text-[10px] font-semibold",
    kind === "Online"
      ? "bg-[color-mix(in_srgb,var(--c7)_15%,transparent)] text-[var(--c7)]"
      : kind === "F2F"
        ? "bg-[color-mix(in_srgb,var(--c3)_15%,transparent)] text-[var(--c3)]"
        : "bg-muted text-foreground",
  );
}

function Chip({ kind }: { kind: EntryKind | null }) {
  return <span className={chipClass(kind)}>{kind ?? "—"}</span>;
}

function ProjectName({ tag }: { tag: Tag | undefined }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <i className="size-2 shrink-0 rounded-sm" style={{ background: tag ? tagColor(tag) : HATCH }} />
      <span className="truncate">{tag?.name ?? UNASSIGNED}</span>
    </span>
  );
}

/** Günün blokları dikey şeritte, proje renginde ve adıyla; atanmamış bloklar taralı. */
function Strip({
  report,
  tags,
  dayStart,
  now,
  hover,
  onHover,
  height,
}: {
  report: Report;
  tags: Map<string, Tag>;
  dayStart: number;
  now: number;
  hover: [number, number] | null;
  onHover: (r: [number, number] | null) => void;
  height: number;
}) {
  const hour = 3600_000;
  const blocks = report.work.blocks.map((b) => ({ b, from: +new Date(b.start), to: +new Date(b.end) }));
  const isToday = isoDate(today()) === isoDate(new Date(dayStart));
  // Şimdi, son bloğa yakınsa (süren iş) şeride girer; akşam uzun bir boşluk şeridi ezmesin.
  const lastEnd = blocks.length ? Math.max(...blocks.map((x) => x.to)) : 0;
  const ends = [...blocks.map((x) => x.to), ...(isToday && now - lastEnd < 2 * hour ? [now] : [])];
  const first = blocks.length ? Math.min(...blocks.map((x) => x.from)) : dayStart + 9 * hour;
  const from = dayStart + Math.floor((first - dayStart) / hour) * hour;
  const to = Math.min(
    dayStart + 24 * hour,
    Math.max(from + 4 * hour, dayStart + Math.ceil((Math.max(first, ...ends) - dayStart) / hour) * hour),
  );
  const hourPx = height / ((to - from) / hour);
  const px = (t: number) => ((t - from) / hour) * hourPx;
  // Sık saatlerde etiketler üst üste binmesin.
  const every = hourPx >= 18 ? 1 : hourPx >= 9 ? 2 : 3;
  const ticks: number[] = [];
  for (let t = from; t <= to; t += every * hour) ticks.push(t);
  return (
    <div className="relative mt-2 ml-9 border-l" style={{ height }} aria-label="Günün blokları">
      {ticks.map((t) => (
        <span
          key={t}
          className="absolute -left-9 -translate-y-1/2 text-[10px] text-muted-foreground tabular"
          style={{ top: px(t) }}
        >
          {hm.format(new Date(t))}
        </span>
      ))}
      {blocks.map(({ b, from: s, to: e }) => {
        const tag = b.projectId ? tags.get(b.projectId) : undefined;
        const lit = !!hover && s < hover[1] && e > hover[0];
        const h = Math.max(3, px(e) - px(s) - 2);
        return (
          <div
            key={b.start}
            onMouseEnter={() => onHover([s, e])}
            onMouseLeave={() => onHover(null)}
            title={`${hm.format(new Date(s))}–${hm.format(new Date(e))} · ${tag?.name ?? UNASSIGNED}`}
            className={cn(
              "absolute right-0 left-1.5 overflow-hidden rounded-md px-1.5 py-0.5 text-[10px] leading-tight font-semibold transition-shadow",
              !tag && "border border-dashed border-muted-foreground/50 text-muted-foreground",
              lit && "ring-2 ring-ring",
            )}
            style={{
              top: px(s) + 1,
              height: h,
              background: tag ? tagColor(tag) : HATCH,
              color: tag ? tagInk(tag) : undefined,
            }}
          >
            {h >= 14 && <span className="block truncate">{tag?.name ?? "Projesiz"}</span>}
          </div>
        );
      })}
      {isToday && now > from && now < to && (
        <span className="absolute -left-1 right-0 h-0.5 rounded-full bg-destructive" style={{ top: px(now) }} />
      )}
    </div>
  );
}
