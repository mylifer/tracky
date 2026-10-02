import type { DayBucket, Tag } from "../api";
import { formatDuration } from "../api";
import { formatShortDay, isoDate } from "../lib/dates";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { Tooltip, useTooltip } from "./Tooltip";

type Props = {
  days: DayBucket[];
  tags: Map<string, Tag>;
  /** Kategori yığın sırası (sabit; renk varlığa bağlı, sıralamaya değil). */
  order: (string | null)[];
  onSelectDay: (isoDate: string) => void;
};

const HEIGHT = 180;

/** Gün başına kategori yığılmış çubuklar; tek eksen (saat). */
export default function WeekChart({ days, tags, order, onSelectDay }: Props) {
  const { tip, show, hide } = useTooltip();
  const maxSecs = Math.max(3600, ...days.map((d) => d.seconds));
  const topHours = Math.ceil(maxSecs / 3600 / 2) * 2;
  const scale = (secs: number) => (secs / (topHours * 3600)) * HEIGHT;
  const ticks = [0, topHours / 2, topHours];

  return (
    <div className="chart week" onMouseLeave={hide}>
      <div className="plot" style={{ height: HEIGHT }}>
        {ticks.map((t) => (
          <div key={t} className="grid" style={{ bottom: scale(t * 3600) }}>
            <span>{t}sa</span>
          </div>
        ))}
        <div className="bars">
          {days.map((d) => {
            const date = new Date(d.start);
            const iso = isoDate(date);
            const byId = new Map(d.categories.map((c) => [c.id, c.seconds]));
            const parts = order.filter((id) => byId.has(id));
            return (
              <button
                key={d.start}
                className="col"
                onClick={() => onSelectDay(iso)}
                aria-label={`${formatShortDay(date)}: ${formatDuration(d.seconds)}`}
              >
                <span className="stack" style={{ height: scale(d.seconds) }}>
                  {parts.map((id) => {
                    const secs = byId.get(id)!;
                    const tag = id ? tags.get(id) : undefined;
                    return (
                      <span
                        key={id ?? "none"}
                        style={{ height: scale(secs), background: tagColor(tag) }}
                        onMouseMove={(e) =>
                          show(
                            e,
                            <>
                              <strong>{formatShortDay(date)}</strong>
                              <div className="tip-row">
                                <i style={{ background: tagColor(tag) }} />
                                {tag?.name ?? UNCATEGORIZED}
                                <b>{formatDuration(secs)}</b>
                              </div>
                              <div className="tip-sub">Gün toplamı {formatDuration(d.seconds)}</div>
                            </>,
                          )
                        }
                      />
                    );
                  })}
                </span>
              </button>
            );
          })}
        </div>
      </div>
      <div className="labels">
        {days.map((d) => (
          <span key={d.start}>
            {formatShortDay(new Date(d.start))}
            <small>{d.seconds > 0 ? formatDuration(d.seconds) : "—"}</small>
          </span>
        ))}
      </div>
      <Tooltip tip={tip} />
    </div>
  );
}

