import { useMemo, useState } from "react";
import type { Segment, Tag, WorkBlock } from "../api";
import { formatDuration } from "../api";
import { addDays, formatTime, isoDate } from "../lib/dates";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { IconBolt } from "./Icons";

const HOUR_PX = 56;
const HOUR_MS = 3600_000;
/** Bu yükseklikten küçük bloklarda yalnızca başlık gösterilir. */
const FULL_LABEL_PX = 44;
/** Bundan alçak bloklarda yazı yok (yalnızca renk; ayrıntı ipucunda). */
const LABEL_MIN_PX = 15;

type Range = { first: number; last: number };

/** Etkinliğe göre gösterilecek saat aralığı (en az 08–18). */
function hourRange(from: Date, spans: { start: string; end: string }[], days = 1): Range {
  let first = 8;
  let last = 18;
  for (const s of spans) {
    const st = new Date(s.start);
    const en = new Date(s.end);
    const dayStart = new Date(st.getFullYear(), st.getMonth(), st.getDate());
    if (days === 1 && +dayStart !== +from) continue;
    first = Math.min(first, st.getHours());
    const endHour = en.getHours() + (en.getMinutes() > 0 ? 1 : 0);
    last = Math.max(last, en.getDate() !== st.getDate() ? 24 : endHour);
  }
  return { first, last: Math.min(24, Math.max(last, first + 6)) };
}

function useTop(from: Date, range: Range) {
  return (t: number) => ((t - (+from + range.first * HOUR_MS)) / HOUR_MS) * HOUR_PX;
}

function HourRail({ range }: { range: Range }) {
  const hours = [];
  for (let h = range.first; h <= range.last; h++) hours.push(h);
  return (
    <div className="cal-rail" style={{ height: (range.last - range.first) * HOUR_PX }}>
      {hours.map((h) => (
        <span key={h} style={{ top: (h - range.first) * HOUR_PX }}>
          {String(h % 24).padStart(2, "0")}:00
        </span>
      ))}
    </div>
  );
}

function GridLines({ range }: { range: Range }) {
  const lines = [];
  for (let h = range.first; h <= range.last; h++) lines.push(h);
  return (
    <>
      {lines.map((h) => (
        <span key={h} className="cal-line" style={{ top: (h - range.first) * HOUR_PX }} />
      ))}
    </>
  );
}

function NowLine({ from, range, days }: { from: Date; range: Range; days: number }) {
  const now = Date.now();
  const end = +addDays(from, days);
  if (now < +from || now >= end) return null;
  const today = new Date();
  const dayStart = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const top = ((now - (+dayStart + range.first * HOUR_MS)) / HOUR_MS) * HOUR_PX;
  if (top < 0 || top > (range.last - range.first) * HOUR_PX) return null;
  return <span className="cal-now" style={{ top }} />;
}

/** Bloğun ayrıntı kartı (Rize'deki gibi uygulama yüzdeleriyle). */
function BlockCard({ block, tags, onClose }: { block: WorkBlock; tags: Map<string, Tag>; onClose: () => void }) {
  const tag = block.categoryId ? tags.get(block.categoryId) : undefined;
  return (
    <div className="pop" role="dialog" aria-label="Oturum ayrıntısı">
      <div className="pop-head">
        <span className="chip" style={{ ["--c" as string]: tagColor(tag) }}>
          <i />
          {tag?.name ?? UNCATEGORIZED}
        </span>
        {block.focus && (
          <span className="focus-chip">
            <IconBolt size={12} /> Odak
          </span>
        )}
        <button className="icon-btn" onClick={onClose} aria-label="Kapat">
          ×
        </button>
      </div>
      <div className="pop-title">
        <strong>{block.topApps[0]?.appName ?? tag?.name ?? "Çalışma"}</strong>
        <span>{formatDuration(block.activeSeconds)}</span>
      </div>
      <p className="muted">
        {formatTime(new Date(block.start))} – {formatTime(new Date(block.end))}
        {block.switches > 0 && ` · ${block.switches} uygulama geçişi`}
      </p>
      <div className="pop-sub">Uygulamalar ve siteler</div>
      <ul className="pct-rows">
        {block.topApps.map((a) => {
          const pct = block.activeSeconds ? Math.round((a.seconds / block.activeSeconds) * 100) : 0;
          return (
            <li key={a.appName}>
              <span className="pct">%{pct}</span>
              <span className="pct-bar">
                <span style={{ width: `${pct}%` }} />
              </span>
              <span className="pct-name">{a.appName}</span>
              <span className="pct-time">{formatDuration(a.seconds)}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function blockTitle(b: WorkBlock, tags: Map<string, Tag>) {
  const tag = b.categoryId ? tags.get(b.categoryId) : undefined;
  return {
    tag,
    title: tag?.name ?? b.topApps[0]?.appName ?? UNCATEGORIZED,
    apps: b.topApps.map((a) => a.appName).join(", "),
  };
}

function Block({
  b,
  tags,
  top,
  height,
  selected,
  onSelect,
}: {
  b: WorkBlock;
  tags: Map<string, Tag>;
  top: number;
  height: number;
  selected: boolean;
  onSelect: () => void;
}) {
  const { tag, title, apps } = blockTitle(b, tags);
  const full = height >= FULL_LABEL_PX;
  const label = height >= LABEL_MIN_PX;
  const summary = `${title} · ${formatTime(new Date(b.start))}–${formatTime(new Date(b.end))} · ${formatDuration(b.activeSeconds)}${apps ? "\n" + apps : ""}`;
  if (!label) {
    return (
      <button
        className={`cal-block bare ${selected ? "sel" : ""}`}
        style={{ top, height, ["--c" as string]: tagColor(tag) }}
        onClick={onSelect}
        title={summary}
        aria-label={summary}
      />
    );
  }
  return (
    <button
      className={`cal-block ${b.focus ? "focus" : ""} ${selected ? "sel" : ""} ${full ? "" : "compact"}`}
      style={{ top, height, ["--c" as string]: tagColor(tag) }}
      onClick={onSelect}
      title={summary}
      aria-label={summary}
    >
      <span className="cb-title">{title}</span>
      {full && (
        <span className="cb-sub">
          {formatTime(new Date(b.start))} – {formatTime(new Date(b.end))}
          {apps && ` · ${apps}`}
        </span>
      )}
      <span className="cb-dur">{formatDuration(b.activeSeconds)}</span>
    </button>
  );
}

/** Gün takvimi: Oturumlar · Uygulamalar · Odak şeridi. */
export function DayCalendar({
  from,
  blocks,
  segments,
  tags,
}: {
  from: Date;
  blocks: WorkBlock[];
  segments: Segment[];
  tags: Map<string, Tag>;
}) {
  const [sel, setSel] = useState<number | null>(null);
  const range = useMemo(() => hourRange(from, [...blocks, ...segments]), [from, blocks, segments]);
  const top = useTop(from, range);
  const height = (range.last - range.first) * HOUR_PX;
  // Kısa uygulama dilimlerini aynı uygulamanın komşularıyla birleştir (takvimde okunur kalsın).
  const apps = useMemo(() => mergeSegments(segments), [segments]);

  return (
    <div className="cal">
      <div className="cal-heads">
        <span />
        <span>Oturumlar</span>
        <span>Uygulamalar</span>
        <span title="Odak blokları" className="focus-head">
          <IconBolt size={13} />
        </span>
      </div>
      <div className="cal-body">
        <HourRail range={range} />
        <div className="cal-col" style={{ height }}>
          <GridLines range={range} />
          {blocks.map((b, i) => {
            const t = top(+new Date(b.start));
            const h = Math.max(4, top(+new Date(b.end)) - t - 2);
            return (
              <Block key={b.start} b={b} tags={tags} top={t} height={h} selected={sel === i} onSelect={() => setSel(sel === i ? null : i)} />
            );
          })}
          <NowLine from={from} range={range} days={1} />
        </div>
        <div className="cal-col" style={{ height }}>
          <GridLines range={range} />
          {apps.map((s, i) => {
            const t = top(+new Date(s.start));
            const h = Math.max(4, top(+new Date(s.end)) - t - 2);
            const tag = s.categoryId ? tags.get(s.categoryId) : undefined;
            return (
              <span
                key={i}
                className={`cal-seg ${h >= 20 ? "" : "thin"}`}
                style={{ top: t, height: h, ["--c" as string]: tagColor(tag) }}
                title={`${s.appName}${s.title ? " — " + s.title : ""}\n${formatTime(new Date(s.start))}–${formatTime(new Date(s.end))} · ${formatDuration((+new Date(s.end) - +new Date(s.start)) / 1000)}`}
              >
                {h >= 20 && <span className="cs-name">{s.appName}</span>}
              </span>
            );
          })}
          <NowLine from={from} range={range} days={1} />
        </div>
        <div className="cal-col focus-col" style={{ height }}>
          {blocks.map((b) => {
            const t = top(+new Date(b.start));
            const h = Math.max(4, top(+new Date(b.end)) - t - 3);
            return <span key={b.start} className={`focus-bar ${b.focus ? "on" : ""}`} style={{ top: t, height: h }} />;
          })}
        </div>
        {sel !== null && blocks[sel] && (
          <div className="pop-anchor" style={{ top: Math.min(top(+new Date(blocks[sel].start)), height - 260) }}>
            <BlockCard block={blocks[sel]} tags={tags} onClose={() => setSel(null)} />
          </div>
        )}
      </div>
    </div>
  );
}

/** Hafta takvimi: her gün bir sütun, bloklar kategori renginde. */
export function WeekCalendar({
  from,
  blocks,
  dayTotals,
  tags,
  onSelectDay,
}: {
  from: Date;
  blocks: WorkBlock[];
  dayTotals: number[];
  tags: Map<string, Tag>;
  onSelectDay: (iso: string) => void;
}) {
  const [sel, setSel] = useState<string | null>(null);
  const range = useMemo(() => hourRange(from, blocks, 7), [from, blocks]);
  const height = (range.last - range.first) * HOUR_PX;
  const days = Array.from({ length: 7 }, (_, i) => addDays(from, i));
  const dayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "short", day: "numeric" });
  const selected = blocks.find((b) => b.start === sel);

  return (
    <div className="cal week-cal">
      <div className="cal-heads">
        <span />
        {days.map((d, i) => (
          <button key={i} className="day-head" onClick={() => onSelectDay(isoDate(d))}>
            {dayFmt.format(d)}
            <small>{dayTotals[i] ? formatDuration(dayTotals[i]) : "—"}</small>
          </button>
        ))}
      </div>
      <div className="cal-body">
        <HourRail range={range} />
        {days.map((d, i) => {
          const dayStart = +d;
          const dayEnd = +addDays(d, 1);
          const top = (t: number) => ((t - (dayStart + range.first * HOUR_MS)) / HOUR_MS) * HOUR_PX;
          return (
            <div key={i} className="cal-col" style={{ height }}>
              <GridLines range={range} />
              {blocks
                .filter((b) => +new Date(b.start) >= dayStart && +new Date(b.start) < dayEnd)
                .map((b) => {
                  const t = top(+new Date(b.start));
                  const h = Math.max(4, top(+new Date(b.end)) - t - 2);
                  return (
                    <Block key={b.start} b={b} tags={tags} top={t} height={h} selected={sel === b.start} onSelect={() => setSel(sel === b.start ? null : b.start)} />
                  );
                })}
              <NowLine from={d} range={range} days={1} />
            </div>
          );
        })}
        {selected && (
          <div className="pop-anchor center">
            <BlockCard block={selected} tags={tags} onClose={() => setSel(null)} />
          </div>
        )}
      </div>
    </div>
  );
}

/** Aynı uygulamanın 2 dakikadan yakın dilimlerini birleştirir. */
function mergeSegments(segments: Segment[]): Segment[] {
  const out: Segment[] = [];
  for (const s of segments) {
    const last = out[out.length - 1];
    if (last && last.appName === s.appName && +new Date(s.start) - +new Date(last.end) < 120_000) {
      last.end = s.end;
    } else {
      out.push({ ...s });
    }
  }
  return out;
}
