import type { DayBucket, Tag } from "../api";
import { formatDuration } from "../api";
import { addDays, isoDate, today } from "../lib/dates";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

const WEEKDAYS = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];

/** Ay takvimi: her gün toplam süre, hedefe göre yoğunluk ve kategori şeridi. */
export default function MonthCalendar({
  from,
  days,
  tags,
  dailyHours,
  onSelectDay,
}: {
  from: Date;
  days: DayBucket[];
  tags: Map<string, Tag>;
  dailyHours: number;
  onSelectDay: (iso: string) => void;
}) {
  const lead = (from.getDay() + 6) % 7;
  const target = Math.max(1, dailyHours * 3600);
  const now = today();

  return (
    <div className="grid grid-cols-7 gap-1.5">
      {WEEKDAYS.map((w) => (
        <div key={w} className="px-1.5 pb-1 text-[11px] font-medium text-muted-foreground">
          {w}
        </div>
      ))}
      {Array.from({ length: lead }, (_, i) => (
        <div key={`b${i}`} />
      ))}
      {days.map((d, i) => {
        const date = addDays(from, i);
        const future = date > now;
        const isToday = +date === +now;
        const level = Math.min(1, d.seconds / target);
        return (
          // Gelecek günler soluk ama açılabilir: o günün toplantıları görünür.
          <button
            key={i}
            onClick={() => onSelectDay(isoDate(date))}
            title={d.seconds ? `${formatDuration(d.seconds)} çalışma` : undefined}
            className={cn(
              "relative flex min-h-[84px] flex-col items-start overflow-hidden rounded-lg border p-2 text-left transition-colors hover:border-primary/60",
              future && "opacity-50",
            )}
            style={{ background: `color-mix(in srgb, var(--primary) ${Math.round(level * 16)}%, var(--card))` }}
          >
            <span
              className={cn(
                "grid size-5 place-items-center rounded-full text-[11px] font-semibold tabular",
                isToday ? "bg-primary text-primary-foreground" : "text-muted-foreground",
              )}
            >
              {date.getDate()}
            </span>
            {d.seconds > 0 && (
              <>
                <span className="mt-auto max-w-full truncate text-[14px] font-semibold tabular">
                  {compact(d.seconds)}
                </span>
                <span className="absolute inset-x-0 bottom-0 flex h-[3px]">
                  {d.categories.map((c) => (
                    <i
                      key={c.id ?? "none"}
                      style={{ flexGrow: c.seconds, background: tagColor(c.id ? tags.get(c.id) : undefined) }}
                    />
                  ))}
                </span>
              </>
            )}
          </button>
        );
      })}
    </div>
  );
}

/** Dar hücreler için: "7,6 sa", "45 dk". */
function compact(secs: number): string {
  if (secs < 3600) return `${Math.max(1, Math.round(secs / 60))} dk`;
  return `${(secs / 3600).toLocaleString("tr-TR", { maximumFractionDigits: 1 })} sa`;
}
