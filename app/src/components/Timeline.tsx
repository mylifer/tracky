import type { Segment, Tag } from "../api";
import { formatDuration } from "../api";
import { formatTime } from "../lib/dates";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { Tooltip, useTooltip } from "./Tooltip";

type Props = { from: Date; segments: Segment[]; tags: Map<string, Tag> };

const HOUR_MS = 3600_000;

/** Günün 24 saatlik şeridi; bloklar kategori rengiyle. Etkinlik yoksa boş şerit. */
export default function Timeline({ from, segments, tags }: Props) {
  const { tip, show, hide } = useTooltip();
  // Etkinliğin olduğu saatlere odaklan (en az 8 saat), boş gece saatleri yer kaplamasın.
  const starts = segments.map((s) => +new Date(s.start));
  const ends = segments.map((s) => +new Date(s.end));
  let a = starts.length ? Math.floor((Math.min(...starts) - +from) / HOUR_MS) : 8;
  let b = ends.length ? Math.ceil((Math.max(...ends) - +from) / HOUR_MS) : 18;
  if (b - a < 8) {
    const pad = Math.ceil((8 - (b - a)) / 2);
    a = Math.max(0, a - pad);
    b = Math.min(24, a + Math.max(8, b - a + pad));
  }
  const t0 = +from + a * HOUR_MS;
  const span = (b - a) * HOUR_MS;
  const pct = (t: number) => ((t - t0) / span) * 100;
  const step = b - a > 12 ? 2 : 1;
  const hours = [];
  for (let h = a; h <= b; h += step) hours.push(h);

  return (
    <div className="chart timeline" onMouseLeave={hide}>
      <div className="track">
        {segments.map((s, i) => {
          const start = +new Date(s.start);
          const end = +new Date(s.end);
          const tag = s.categoryId ? tags.get(s.categoryId) : undefined;
          return (
            <span
              key={i}
              className="seg"
              style={{
                left: `${pct(start)}%`,
                width: `max(2px, calc(${pct(end) - pct(start)}% - 1px))`,
                background: tagColor(tag),
              }}
              onMouseMove={(e) =>
                show(
                  e,
                  <>
                    <strong>{s.appName}</strong>
                    {s.title && <div className="tip-sub">{s.title}</div>}
                    <div className="tip-row">
                      <i style={{ background: tagColor(tag) }} />
                      {tag?.name ?? UNCATEGORIZED}
                    </div>
                    <div className="tip-sub">
                      {formatTime(new Date(start))}–{formatTime(new Date(end))} ·{" "}
                      {formatDuration((end - start) / 1000)}
                    </div>
                  </>,
                )
              }
            />
          );
        })}
      </div>
      <div className="axis">
        {hours.map((h) => (
          <span key={h} style={{ left: `${pct(+from + h * HOUR_MS)}%` }}>
            {String(h % 24).padStart(2, "0")}
          </span>
        ))}
      </div>
      <Tooltip tip={tip} />
    </div>
  );
}
