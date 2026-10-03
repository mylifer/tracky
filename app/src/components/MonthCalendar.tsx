import type { DayBucket, Tag } from "../api";
import { formatDuration } from "../api";
import { addDays, isoDate, today } from "../lib/dates";
import { tagColor } from "../lib/tags";

const WEEKDAYS = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];

/** Ay takvimi: her gün toplam süre, yoğunluk ve kategori şeridiyle. */
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
    <div className="month-cal">
      {WEEKDAYS.map((w) => (
        <div key={w} className="mc-head">
          {w}
        </div>
      ))}
      {Array.from({ length: lead }, (_, i) => (
        <div key={`b${i}`} className="mc-blank" />
      ))}
      {days.map((d, i) => {
        const date = addDays(from, i);
        const future = date > now;
        const level = Math.min(1, d.seconds / target);
        return (
          <button
            key={i}
            className={`mc-day ${+date === +now ? "today" : ""}`}
            disabled={future}
            onClick={() => onSelectDay(isoDate(date))}
            style={{ "--level": level } as React.CSSProperties}
            title={d.seconds ? `${formatDuration(d.seconds)} çalışma · ${formatDuration(d.focusSeconds)} odak · skor ${d.focusScore}` : undefined}
          >
            <span className="mc-num">{date.getDate()}</span>
            {d.seconds > 0 && (
              <>
                <span className="mc-total">{compact(d.seconds)}</span>
                {d.focusSeconds > 0 && <span className="mc-focus">{compact(d.focusSeconds)}</span>}
                <span className="mc-bar">
                  {d.categories.map((c) => (
                    <i
                      key={c.id ?? "none"}
                      style={{
                        flexGrow: c.seconds,
                        background: tagColor(c.id ? tags.get(c.id) : undefined),
                      }}
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
