import { useEffect, useMemo, useState } from "react";
import { Plus, Search as SearchIcon } from "lucide-react";
import { api, formatDuration, type Tag, type Trends as TrendsData } from "../api";
import { ErrorText, Page } from "../components/settings";
import { Button } from "../components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addDays, formatDate } from "../lib/dates";
import { UNCATEGORIZED, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";

const WEEK_OPTIONS = [8, 12, 26] as const;
type Kind = "projects" | "categories";

/**
 * Eğilimler: proje ve kategorilerin son haftalardaki süresi. Her satır kendi renginde küçük bir
 * haftalık çubuk grafik; tüm satırlar aynı ölçekte (büyüklükler kıyaslanabilsin). Süren hafta
 * yarım olduğundan soluk çizilir ve ortalamaya katılmaz.
 */
export default function Trends({
  onSearch,
  onAddProject,
}: {
  onSearch: (query: string) => void;
  onAddProject: () => void;
}) {
  const [weeks, setWeeks] = useState<number>(8);
  const [kind, setKind] = useState<Kind | null>(null);
  const [data, setData] = useState<TrendsData | null>(null);
  const [tags, setTags] = useState<Map<string, Tag>>(new Map());
  // Proje → aranacak sözcük: ilk başlık kuralının deseni (proje adı kuralla aynı olmayabilir).
  const [patterns, setPatterns] = useState<Map<string, string>>(new Map());
  // Proje → haftalık hedef (saniye).
  const [targets, setTargets] = useState<Map<string, number>>(new Map());
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([api.trends(weeks), api.taxonomy(), api.goals()]).then(
      ([d, t, g]) => {
        setTargets(new Map((g.projectGoals ?? []).map((x) => [x.projectId, x.minutes * 60])));
        if (!live) return;
        setData(d);
        setTags(new Map(t.tags.map((x) => [x.id, x])));
        const first = new Map<string, string>();
        for (const r of t.rules) if (r.field === "title" && !first.has(r.tagId)) first.set(r.tagId, r.pattern);
        setPatterns(first);
        setError(null);
      },
      (e) => live && setError(friendlyError(e)),
    );
    return () => {
      live = false;
    };
  }, [weeks]);

  // Proje yoksa kategorilerle açılır.
  const shownKind: Kind = kind ?? (data && data.projects.length === 0 ? "categories" : "projects");
  const series = data ? (shownKind === "projects" ? data.projects : data.categories) : [];
  const max = useMemo(() => Math.max(1, ...series.flatMap((s) => s.seconds)), [series]);

  return (
    <Page title="Eğilimler">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Tabs value={shownKind} onValueChange={(v) => setKind(v as Kind)}>
          <TabsList aria-label="Tür">
            <TabsTrigger value="projects" className="px-3">
              Projeler
            </TabsTrigger>
            <TabsTrigger value="categories" className="px-3">
              Kategoriler
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <Tabs value={String(weeks)} onValueChange={(v) => setWeeks(Number(v))}>
          <TabsList aria-label="Dönem">
            {WEEK_OPTIONS.map((w) => (
              <TabsTrigger key={w} value={String(w)} className="px-3">
                {w} hafta
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <ErrorText>{error}</ErrorText>
      {data && data.projects.length === 0 && shownKind === "categories" && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-dashed px-4 py-2.5 text-xs text-muted-foreground">
          <span>Henüz proje yok: proje ya da müşteri bazında süreyi görmek için bir proje ekle.</span>
          <Button size="sm" variant="outline" onClick={onAddProject}>
            <Plus /> Proje ekle
          </Button>
        </div>
      )}

      {data && series.length === 0 ? (
        shownKind === "projects" ? (
          <div className="space-y-3 rounded-xl border bg-card px-5 py-4 shadow-xs">
            <p className="text-sm text-muted-foreground">
              Henüz bir projeye düşen süre yok. Proje, pencere başlığında geçen bir sözcükle (örn. proje ya da müşteri
              adı) uygulamalar arası çalışmayı toplar.
            </p>
            <Button size="sm" onClick={onAddProject}>
              <Plus /> Proje ekle
            </Button>
          </div>
        ) : (
          <p className="px-1 text-sm text-muted-foreground">Bu dönemde kayıt yok.</p>
        )
      ) : data ? (
        <section className="space-y-2">
          <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)_88px_88px] items-end gap-3 px-4 text-[11px] text-muted-foreground">
            <span>{shownKind === "projects" ? "Proje" : "Kategori"}</span>
            <span className="flex justify-between tabular">
              <span>{formatDate(new Date(data.periods[0]))}</span>
              <span>bu hafta</span>
            </span>
            <span className="text-right">Bu hafta</span>
            <span className="text-right" title="İlk kayıttan bu yana tamamlanmış haftaların ortalaması">
              Haftalık ort.
            </span>
          </div>
          <ul className="divide-y rounded-xl border bg-card shadow-xs">
            {series.map((s) => {
              const tag = s.id ? tags.get(s.id) : undefined;
              const name = tag?.name ?? (shownKind === "projects" ? "Projesiz" : UNCATEGORIZED);
              return (
                <Row
                  key={s.id ?? "\u0000none"}
                  name={name}
                  color={tagColor(tag)}
                  seconds={s.seconds}
                  periods={data.periods}
                  max={max}
                  target={shownKind === "projects" && s.id ? targets.get(s.id) : undefined}
                  // Kategoriler uygulama kurallarıyla tanımlı: adlarıyla aramak bir şey bulmaz.
                  onSearch={tag?.kind === "project" ? () => onSearch(patterns.get(tag.id) ?? tag.name) : undefined}
                />
              );
            })}
          </ul>
          <p className="px-1 text-xs text-muted-foreground">
            Çubuklar haftalık süre (tüm satırlarda aynı ölçek); soluk olan süren hafta.
            {shownKind === "projects" && " Proje adına tıklayınca o projenin pencerelerini arar."}
          </p>
        </section>
      ) : null}
    </Page>
  );
}

function Row({
  name,
  color,
  seconds,
  periods,
  max,
  target,
  onSearch,
}: {
  name: string;
  color: string;
  seconds: number[];
  periods: string[];
  max: number;
  /** Projenin haftalık hedefi (saniye): "bu hafta" sütununda ilerleme gösterilir. */
  target?: number;
  onSearch?: () => void;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const last = seconds.length - 1;
  // Ortalama, ilk süre görülen haftadan itibaren tamamlanmış haftalar: proje sonradan
  // başladıysa öncesindeki boş haftalar ortalamayı düşürmesin.
  const firstActive = seconds.findIndex((v) => v > 0);
  const done = firstActive < 0 ? [] : seconds.slice(firstActive, last);
  const avg = done.length ? done.reduce((a, b) => a + b, 0) / done.length : 0;
  const weekLabel = (i: number) => {
    const start = new Date(periods[i]);
    return `${formatDate(start)} – ${formatDate(addDays(start, 6))}`;
  };

  return (
    <li className="grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)_88px_88px] items-center gap-3 px-4 py-2.5">
      <span className="flex min-w-0 items-center gap-2">
        <i className="size-2 shrink-0 rounded-full" style={{ background: color }} />
        {onSearch ? (
          <button
            className="group flex min-w-0 items-center gap-1 text-left text-[13px] font-medium hover:underline"
            onClick={onSearch}
            title="Bu projenin pencerelerini ara"
          >
            <span className="truncate">{name}</span>
            <SearchIcon className="size-3 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100" />
          </button>
        ) : (
          <span className="truncate text-[13px] font-medium">{name}</span>
        )}
      </span>
      <span className="flex h-9 items-end gap-[2px]" onMouseLeave={() => setHover(null)}>
        {seconds.map((v, i) => (
          <span
            key={i}
            className="flex h-full min-w-0 flex-1 items-end"
            onMouseEnter={() => setHover(i)}
            aria-label={`${weekLabel(i)}: ${formatDuration(v)}`}
          >
            <span
              className={cn(
                "block w-full rounded-t-[3px] transition-opacity",
                i === last && "opacity-45",
                hover !== null && hover !== i && "opacity-35",
              )}
              style={{ height: v ? `${Math.max(6, (v / max) * 100)}%` : 0, background: color }}
            />
          </span>
        ))}
      </span>
      {hover !== null ? (
        <span className="col-span-2 text-right text-xs tabular">
          <span className="text-muted-foreground">{hover === last ? "bu hafta" : weekLabel(hover)} · </span>
          {formatDuration(seconds[hover])}
        </span>
      ) : (
        <>
          <span className="text-right text-[13px] tabular">
            {formatDuration(seconds[last] ?? 0)}
            {target ? (
              <span className="block" title={`Haftalık hedef: ${formatDuration(target)}`}>
                <span className="text-[11px] text-muted-foreground">/ {formatDuration(target)}</span>
                <span className="mt-0.5 ml-auto block h-1 w-14 overflow-hidden rounded-full bg-muted">
                  <span
                    className="block h-full rounded-full"
                    style={{ width: `${Math.min(100, ((seconds[last] ?? 0) / target) * 100)}%`, background: color }}
                  />
                </span>
              </span>
            ) : null}
          </span>
          <span className="text-right text-[13px] text-muted-foreground tabular">
            {formatDuration(Math.round(avg))}
          </span>
        </>
      )}
    </li>
  );
}
