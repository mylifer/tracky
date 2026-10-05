import { CircleCheck, TriangleAlert } from "lucide-react";
import { addDays, isoDate, parseIsoDate } from "../lib/dates";
import { divisionColor, type DaySummary } from "../lib/timesheet";
import { cn } from "../lib/utils";

/**
 * Zaman çizelgesinin hafta ve ay panoları: dönemin günleri bir bakışta (saat, birimlere dağılım,
 * gönderim durumu, bakılacaklar). Güne tıklayınca o günün satırları panonun altında açılır.
 */

const hoursFmt = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 0, maximumFractionDigits: 2 });
const weekdayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short" });
const WEEKDAYS = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];

const isWeekend = (iso: string) => {
  const d = parseIsoDate(iso).getDay();
  return d === 0 || d === 6;
};

/** "6,25" ya da boş günde "—". */
function hoursText(h: number) {
  return h > 0 ? hoursFmt.format(h) : "—";
}

/** Günün durumu: gönderildi, bekleyen satır ya da bakılacak bir şey var. */
function dayTip(s: DaySummary, dayHours: number) {
  const lines = [
    s.empty ? "Kayıt yok" : `${hoursFmt.format(s.hours)} sa`,
    ...s.divisions.map((d) => `${d.division}: ${hoursFmt.format(d.hours)} sa`),
  ];
  if (s.sent) lines.push("Hepsi gönderildi");
  else if (s.unsent) lines.push(`${s.unsent} satır gönderilmedi`);
  if (s.diff)
    lines.push(
      `Günlük ${hoursFmt.format(dayHours)} saatten ${s.diff < 0 ? "eksik" : "fazla"}: ${hoursFmt.format(Math.abs(s.diff))} sa`,
    );
  lines.push(...s.problems);
  return lines.join("\n");
}

/** Günün durum simgesi: sorun > bekleyen > gönderildi. */
function StatusMark({ s, compact = false }: { s: DaySummary; compact?: boolean }) {
  if (s.problems.length || s.diff)
    return (
      <span className="flex items-center gap-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-400">
        <TriangleAlert className="size-3" aria-hidden />
        {!compact && (s.problems.length || "")}
      </span>
    );
  if (s.unsent)
    return (
      <span className="flex items-center gap-1 text-[10px] font-medium text-primary tabular">
        <i className="size-1.5 rounded-full bg-primary" aria-hidden />
        {s.unsent}
        {!compact && " bekliyor"}
      </span>
    );
  if (s.sent) return <CircleCheck className="size-3 text-success" aria-label="Gönderildi" />;
  return null;
}

/** Günlük saatin doluluğu: tutuyorsa yeşil (gönderildiyse) ya da mavi, sapma varsa sarı. */
function Fill({ s, dayHours }: { s: DaySummary; dayHours: number }) {
  const pct = Math.min(1, s.hours / (dayHours || 8)) * 100;
  const tone = s.diff ? "bg-amber-500" : s.sent ? "bg-success" : "bg-primary";
  return (
    <span className="block h-1 w-full overflow-hidden rounded-full bg-muted">
      <span className={cn("block h-full rounded-full", tone)} style={{ width: `${pct}%` }} />
    </span>
  );
}

/** Birimlerin günün saatindeki payı (günlük saate göre; fazlası tam dolar). */
function DivisionBar({ s, divisions, dayHours }: { s: DaySummary; divisions: string[]; dayHours: number }) {
  const scale = Math.max(dayHours || 8, s.hours);
  return (
    <span className="flex h-1.5 w-full overflow-hidden rounded-full bg-muted">
      {s.divisions.map((d) => (
        <span
          key={d.division}
          className="h-full"
          style={{ width: `${(d.hours / scale) * 100}%`, background: divisionColor(divisions, d.division) }}
        />
      ))}
    </span>
  );
}

type BoardProps = {
  summaries: DaySummary[];
  /** Birimlerin renk sırası. */
  divisions: string[];
  dayHours: number;
  selected: string | null;
  onSelect: (iso: string) => void;
  todayIso: string;
};

/** Dönemdeki birimler, toplam saate göre (renk sırası `divisions`'tan). */
function periodDivisions(summaries: DaySummary[]) {
  const totals = new Map<string, number>();
  for (const s of summaries)
    for (const d of s.divisions) totals.set(d.division, (totals.get(d.division) ?? 0) + d.hours);
  return [...totals.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], "tr"));
}

/**
 * Hafta panosu: birim × gün saat tablosu. Gün başlığında günün saati, günlük saate göre doluluğu
 * ve durumu; satırlarda birimlerin günlük saati, sonda toplamlar.
 */
export function WeekBoard({ summaries, divisions, dayHours, selected, onSelect, todayIso }: BoardProps) {
  const rows = periodDivisions(summaries);
  const total = summaries.reduce((s, d) => s + d.hours, 0);
  const grid = "grid grid-cols-[minmax(110px,1.3fr)_repeat(7,minmax(0,1fr))_minmax(64px,0.8fr)]";
  const col = (iso: string) =>
    cn(
      "transition-colors",
      iso === selected ? "bg-accent" : iso === todayIso ? "bg-primary/[0.04]" : isWeekend(iso) && "bg-muted/30",
    );
  return (
    <section className="overflow-x-auto rounded-xl border bg-card shadow-xs" aria-label="Haftanın özeti">
      <div className="min-w-[640px]">
        <div className={cn(grid, "border-b")}>
          <span className="self-end px-4 pb-2 text-[11px] text-muted-foreground">Birim</span>
          {summaries.map((s) => {
            const d = parseIsoDate(s.date);
            return (
              <button
                key={s.date}
                className={cn(
                  col(s.date),
                  "flex flex-col gap-1 px-2 pt-2.5 pb-2 text-left hover:bg-accent/70 focus-visible:outline-2 focus-visible:outline-ring",
                )}
                aria-pressed={s.date === selected}
                title={dayTip(s, dayHours)}
                onClick={() => onSelect(s.date)}
              >
                <span className="flex items-center gap-1 text-[11px] text-muted-foreground capitalize">
                  {weekdayFmt.format(d)}
                  <span
                    className={cn(
                      "inline-grid size-5 place-items-center rounded-full text-[11px] font-semibold tabular",
                      s.date === todayIso ? "bg-primary text-primary-foreground" : "text-foreground",
                    )}
                  >
                    {d.getDate()}
                  </span>
                  <span className="ml-auto">
                    <StatusMark s={s} compact />
                  </span>
                </span>
                <span className={cn("text-sm font-semibold tabular", s.hours === 0 && "text-muted-foreground")}>
                  {hoursText(s.hours)}
                </span>
                {isWeekend(s.date) && s.hours === 0 ? <span className="h-1" /> : <Fill s={s} dayHours={dayHours} />}
              </button>
            );
          })}
          <span className="flex flex-col justify-end gap-1 px-3 pb-2 text-right">
            <span className="text-[11px] text-muted-foreground">Toplam</span>
            <span className="text-sm font-semibold tabular">{hoursText(total)}</span>
            <span className="text-[10px] text-muted-foreground tabular">
              {hoursFmt.format(total / (dayHours || 8))} ag
            </span>
          </span>
        </div>
        {rows.length === 0 ? (
          <p className="px-4 py-3 text-xs text-muted-foreground">
            Bu hafta kayıt yok. Raporda bu çizelgenin projelerine atadığın süre burada satır olur.
          </p>
        ) : (
          rows.map(([division, sum]) => (
            <div key={division} className={cn(grid, "border-b last:border-b-0")}>
              <span className="flex min-w-0 items-center gap-2 px-4 py-1.5 text-xs">
                <i
                  className="size-2 shrink-0 rounded-full"
                  style={{ background: divisionColor(divisions, division) }}
                />
                <span className="truncate" title={division}>
                  {division || "(birimsiz)"}
                </span>
              </span>
              {summaries.map((s) => {
                const h = s.divisions.find((d) => d.division === division)?.hours ?? 0;
                return (
                  <button
                    key={s.date}
                    tabIndex={-1}
                    className={cn(col(s.date), "px-2 py-1.5 text-left text-xs tabular hover:bg-accent/70")}
                    onClick={() => onSelect(s.date)}
                  >
                    {h > 0 ? hoursFmt.format(h) : ""}
                  </button>
                );
              })}
              <span className="px-3 py-1.5 text-right text-xs font-medium tabular">{hoursFmt.format(sum)}</span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

/**
 * Ay panosu: takvim ızgarası. Her günde saat, birimlerin payı ve durum; satır sonunda haftanın
 * toplamı. Üstte ayın birim toplamları.
 */
export function MonthBoard({ summaries, divisions, dayHours, selected, onSelect, todayIso }: BoardProps) {
  if (summaries.length === 0) return null;
  const first = parseIsoDate(summaries[0].date);
  const lead = (first.getDay() + 6) % 7;
  const cells: (DaySummary | null)[] = [...Array<null>(lead).fill(null), ...summaries];
  while (cells.length % 7) cells.push(null);
  const weeks: (DaySummary | null)[][] = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  const totals = periodDivisions(summaries);
  const total = summaries.reduce((s, d) => s + d.hours, 0);
  const grid = "grid grid-cols-[repeat(7,minmax(0,1fr))_minmax(68px,0.7fr)] gap-1.5";

  return (
    <section className="space-y-2 rounded-xl border bg-card p-3 shadow-xs" aria-label="Ayın özeti">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-1 text-[11px] text-muted-foreground">
        {totals.map(([division, h]) => (
          <span key={division} className="flex items-center gap-1.5">
            <i className="size-2 rounded-full" style={{ background: divisionColor(divisions, division) }} />
            <span className="text-foreground">{division || "(birimsiz)"}</span>
            <span className="tabular">{hoursFmt.format(h)} sa</span>
          </span>
        ))}
        {totals.length === 0 && <span>Bu ay kayıt yok.</span>}
        <span className="ml-auto flex items-center gap-3">
          <span className="flex items-center gap-1">
            <CircleCheck className="size-3 text-success" /> gönderildi
          </span>
          <span className="flex items-center gap-1">
            <i className="size-1.5 rounded-full bg-primary" /> bekliyor
          </span>
          <span className="flex items-center gap-1">
            <TriangleAlert className="size-3 text-amber-600 dark:text-amber-400" /> bakılacak
          </span>
        </span>
      </div>
      <div className={cn(grid, "px-0.5 text-[11px] text-muted-foreground")}>
        {WEEKDAYS.map((w) => (
          <span key={w} className="px-1">
            {w}
          </span>
        ))}
        <span className="px-1 text-right">Hafta</span>
      </div>
      {weeks.map((week, i) => {
        const days = week.filter((d): d is DaySummary => !!d);
        const sum = days.reduce((s, d) => s + d.hours, 0);
        const monday = isoDate(addDays(first, i * 7 - lead));
        return (
          <div key={monday} className={grid}>
            {week.map((s, j) =>
              s ? (
                <MonthCell
                  key={s.date}
                  s={s}
                  divisions={divisions}
                  dayHours={dayHours}
                  selected={s.date === selected}
                  today={s.date === todayIso}
                  onSelect={onSelect}
                />
              ) : (
                <span key={`bos-${j}`} />
              ),
            )}
            <span className="flex flex-col items-end justify-center px-1 text-right">
              <span className={cn("text-xs font-semibold tabular", sum === 0 && "text-muted-foreground")}>
                {hoursText(sum)}
              </span>
              {sum > 0 && (
                <span className="text-[10px] text-muted-foreground tabular">
                  {hoursFmt.format(sum / (dayHours || 8))} ag
                </span>
              )}
            </span>
          </div>
        );
      })}
      <div className="flex justify-end gap-2 px-1 pt-1 text-xs">
        <span className="text-muted-foreground">Ay toplamı</span>
        <span className="font-semibold tabular">{hoursFmt.format(total)} sa</span>
        <span className="text-muted-foreground tabular">· {hoursFmt.format(total / (dayHours || 8))} ag</span>
      </div>
    </section>
  );
}

function MonthCell({
  s,
  divisions,
  dayHours,
  selected,
  today,
  onSelect,
}: {
  s: DaySummary;
  divisions: string[];
  dayHours: number;
  selected: boolean;
  today: boolean;
  onSelect: (iso: string) => void;
}) {
  const d = parseIsoDate(s.date);
  const weekend = isWeekend(s.date);
  return (
    <button
      className={cn(
        "flex min-h-[78px] flex-col gap-1 rounded-lg border p-1.5 text-left transition-colors hover:border-foreground/25 focus-visible:outline-2 focus-visible:outline-ring",
        weekend && s.empty ? "border-transparent bg-muted/30" : "bg-card",
        selected && "border-primary ring-1 ring-primary",
      )}
      aria-pressed={selected}
      title={dayTip(s, dayHours)}
      onClick={() => onSelect(s.date)}
    >
      <span className="flex items-center justify-between gap-1">
        <span
          className={cn(
            "inline-grid size-5 place-items-center rounded-full text-[11px] font-semibold tabular",
            today ? "bg-primary text-primary-foreground" : weekend ? "text-muted-foreground" : "text-foreground",
          )}
        >
          {d.getDate()}
        </span>
        <StatusMark s={s} compact />
      </span>
      <span
        className={cn(
          "mt-auto text-[13px] leading-none font-semibold tabular",
          s.hours === 0 && "font-normal text-muted-foreground",
        )}
      >
        {hoursText(s.hours)}
        {s.hours > 0 && <span className="ml-0.5 text-[10px] font-normal text-muted-foreground">sa</span>}
      </span>
      {s.hours > 0 ? <DivisionBar s={s} divisions={divisions} dayHours={dayHours} /> : <span className="h-1.5" />}
    </button>
  );
}
