import { useEffect, useMemo, useRef, useState } from "react";
import { FileDown, Search as SearchIcon } from "lucide-react";
import { api, formatDuration, type SearchResult } from "../api";
import { ErrorText, Page } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addDays, formatDate, isoDate, today } from "../lib/dates";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";

export const PERIODS = [
  { days: 7, label: "7 gün" },
  { days: 30, label: "30 gün" },
  { days: 90, label: "90 gün" },
  { days: 365, label: "1 yıl" },
] as const;

/** Bundan uzun dönemlerde çubuklar haftalara toplanır (365 çubuk okunmaz). */
const DAILY_MAX = 90;

export type SearchState = { query: string; days: number };

/** Başlıkta ya da uygulama adında geçen ifadeye göre süre: toplam, dağılım, uygulamalar, başlıklar. */
export default function Search({
  state,
  onChange,
  onSelectDay,
}: {
  state: SearchState;
  onChange: (s: SearchState) => void;
  onSelectDay: (iso: string) => void;
}) {
  // Sonuç hangi dönemin olduğuyla tutulur: dönem değişince eski sonuç yeni tarihlere çizilmesin.
  const [found, setFound] = useState<{ days: number; result: SearchResult } | null>(null);
  const result = found?.days === state.days ? found.result : null;
  const [error, setError] = useState<string | null>(null);
  const start = useMemo(() => addDays(today(), 1 - state.days), [state.days]);

  // Yazarken her tuşta değil, kısa bir duraklamadan sonra ara; geç gelen eski yanıt ezmesin.
  const seq = useRef(0);
  useEffect(() => {
    const query = state.query.trim();
    const n = ++seq.current;
    if (!query) {
      setFound(null);
      return;
    }
    const days = state.days;
    const id = window.setTimeout(() => {
      api.search(query, isoDate(start), state.days).then(
        (r) => {
          if (n !== seq.current) return;
          setFound({ days, result: r });
          setError(null);
        },
        (e) => n === seq.current && setError(friendlyError(e)),
      );
    }, 250);
    return () => window.clearTimeout(id);
  }, [state.query, state.days, start]);

  return (
    <Page title="Ara">
      <div className="space-y-3">
        <div className="relative">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            autoFocus
            type="search"
            value={state.query}
            onChange={(e) => onChange({ ...state, query: e.target.value })}
            placeholder="Pencere başlığında ya da uygulama adında ara (örn. tracky, Figma, toplantı)"
            className="h-10 pl-9 text-sm"
            aria-label="Arama"
          />
        </div>
        <Tabs value={String(state.days)} onValueChange={(v) => onChange({ ...state, days: Number(v) })}>
          <TabsList aria-label="Dönem">
            {PERIODS.map((p) => (
              <TabsTrigger key={p.days} value={String(p.days)} className="px-3">
                Son {p.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>

      <ErrorText>{error}</ErrorText>
      {!state.query.trim() ? (
        <p className="px-1 text-sm text-muted-foreground">
          Bir proje, müşteri ya da konu adı yaz: o sözcüğün geçtiği pencerelerde ve uygulamalarda geçen süreyi gösterir.
          Büyük/küçük harf ve ı/i farkı önemsizdir.
        </p>
      ) : result && result.totalSeconds === 0 ? (
        <p className="px-1 text-sm text-muted-foreground">
          Son {state.days} günde “{state.query.trim()}” geçen bir kayıt yok.
        </p>
      ) : result ? (
        <Results
          result={result}
          start={start}
          onSelectDay={onSelectDay}
          onExport={() => api.exportSearch(state.query.trim(), isoDate(start), state.days)}
        />
      ) : null}
    </Page>
  );
}

function Results({
  result,
  start,
  onSelectDay,
  onExport,
}: {
  result: SearchResult;
  start: Date;
  onSelectDay: (iso: string) => void;
  /** CSV'yi yazar, dosya yolunu döndürür. */
  onExport: () => Promise<string>;
}) {
  const [saved, setSaved] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  // Başka bir arama için eski "Kaydedildi" mesajı kalmasın.
  useEffect(() => {
    setSaved(null);
    setExportError(null);
  }, [result]);
  const active = result.days.filter((s) => s > 0).length;
  const maxApp = result.apps[0]?.seconds || 1;
  return (
    <div className="space-y-6">
      <section className="rounded-xl border bg-card px-5 py-4 shadow-xs">
        <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
          <span className="text-3xl font-semibold tracking-tight tabular">{formatDuration(result.totalSeconds)}</span>
          <span className="text-sm text-muted-foreground">
            {active} günde · çalışılan gün başına{" "}
            {formatDuration(Math.round(result.totalSeconds / Math.max(1, active)))}
          </span>
        </div>
        <Bars days={result.days} start={start} onSelectDay={onSelectDay} />
        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 border-t pt-3">
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              onExport().then(
                (path) => {
                  setSaved(path);
                  setExportError(null);
                },
                (e) => setExportError(friendlyError(e)),
              )
            }
          >
            <FileDown /> CSV olarak dışa aktar
          </Button>
          <span className="min-w-0 truncate text-xs text-muted-foreground selectable" title={saved ?? undefined}>
            {exportError ? (
              <span className="text-destructive">{exportError}</span>
            ) : saved ? (
              `Kaydedildi: ${saved}`
            ) : (
              "Eşleşen oturumlar; faturalama ya da Excel için"
            )}
          </span>
        </div>
      </section>

      <section className="space-y-2">
        <h2 className="px-1 text-[13px] font-semibold">Uygulamalar</h2>
        <ul className="divide-y rounded-xl border bg-card shadow-xs">
          {result.apps.map((a) => (
            <li key={a.appId} className="flex items-center gap-3 px-4 py-2.5">
              <span className="w-40 shrink-0 truncate text-[13px] font-medium">{a.appName}</span>
              <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                <span
                  className="block h-full rounded-full bg-primary"
                  style={{ width: `${Math.max(2, (a.seconds / maxApp) * 100)}%` }}
                />
              </span>
              <span className="w-[72px] shrink-0 text-right text-[13px] tabular">{formatDuration(a.seconds)}</span>
            </li>
          ))}
        </ul>
      </section>

      <section className="space-y-2">
        <h2 className="px-1 text-[13px] font-semibold">Pencereler</h2>
        <ul className="divide-y rounded-xl border bg-card shadow-xs">
          {result.titles.map((t) => (
            <li key={`${t.appName}\u0000${t.title}`} className="flex items-center gap-3 px-4 py-2">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] selectable" title={t.title}>
                  {t.title || "(başlıksız)"}
                </span>
                <span className="block text-xs text-muted-foreground">{t.appName}</span>
              </span>
              <span className="shrink-0 text-[13px] text-muted-foreground tabular">{formatDuration(t.seconds)}</span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

type Bucket = { start: Date; days: number; seconds: number };

/** Günlük (uzun dönemde haftalık) dağılım; çubuğa tıklayınca o güne/haftanın ilk gününe gider. */
function Bars({ days, start, onSelectDay }: { days: number[]; start: Date; onSelectDay: (iso: string) => void }) {
  const [hover, setHover] = useState<number | null>(null);
  const buckets = useMemo<Bucket[]>(() => {
    const size = days.length > DAILY_MAX ? 7 : 1;
    const out: Bucket[] = [];
    for (let i = 0; i < days.length; i += size) {
      const part = days.slice(i, i + size);
      out.push({ start: addDays(start, i), days: part.length, seconds: part.reduce((a, b) => a + b, 0) });
    }
    return out;
  }, [days, start]);
  const max = Math.max(...buckets.map((b) => b.seconds), 1);
  const weekly = buckets[0]?.days > 1;
  const label = (b: Bucket) =>
    b.days > 1 ? `${formatDate(b.start)} – ${formatDate(addDays(b.start, b.days - 1))}` : formatDate(b.start);
  const shown = hover !== null ? buckets[hover] : null;

  return (
    <div className="mt-4">
      <div className="mb-1 h-4 text-xs text-muted-foreground tabular">
        {shown ? (
          <>
            <span className="text-foreground">{label(shown)}</span> · {formatDuration(shown.seconds)}
          </>
        ) : weekly ? (
          "Haftalık dağılım · çubuğa tıklayınca haftanın ilk gününe gider"
        ) : (
          "Günlük dağılım · çubuğa tıklayınca o güne gider"
        )}
      </div>
      <div className="flex h-24 items-end gap-[2px] border-b border-border" onMouseLeave={() => setHover(null)}>
        {buckets.map((b, i) => (
          <button
            key={i}
            className="group flex h-full min-w-0 flex-1 items-end"
            onMouseEnter={() => setHover(i)}
            onFocus={() => setHover(i)}
            onClick={() => onSelectDay(isoDate(b.start))}
            aria-label={`${label(b)}: ${formatDuration(b.seconds)}`}
          >
            <span
              className={cn(
                "block w-full rounded-t-[4px] bg-primary transition-opacity",
                hover !== null && hover !== i && "opacity-45",
                b.seconds === 0 && "opacity-0",
              )}
              style={{ height: `${b.seconds ? Math.max(4, (b.seconds / max) * 100) : 0}%` }}
            />
          </button>
        ))}
      </div>
      <div className="mt-1 flex justify-between text-[11px] text-muted-foreground tabular">
        <span>{formatDate(buckets[0]?.start ?? start)}</span>
        <span>{formatDate(addDays(start, days.length - 1))}</span>
      </div>
    </div>
  );
}
