import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Hourglass, ZoomIn, ZoomOut } from "lucide-react";
import { api, type CategoryLimit, type ProjectGoal, type Report, type Tag } from "../api";
import { addDays, addMonths, daysInMonth, isoDate, parseIsoDate, today } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { useTauriEvent } from "../lib/useTauriEvent";
import { clampZoom, stepZoom, useZoomGestures } from "../lib/zoom";
import { AppList, Legend } from "./Breakdown";
import AppTimeline from "./AppTimeline";
import { DayCalendar, HOUR_PX, WeekCalendar } from "./Calendar";
import MonthCalendar from "./MonthCalendar";
import { EditContext, type EntryDraft, ManualEntry, RangeMenu, type RangeSelection } from "./SessionEdit";
import Summary from "./Summary";
import Toolbar from "./Toolbar";
import { Button } from "./ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "./ui/card";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

export type Mode = "day" | "week" | "month";

/** Süren dönemde raporun canlı yenilenme aralığı. */
const LIVE_REFRESH_MS = 15_000;

const MODES: { id: Mode; label: string; current: string; summary: string }[] = [
  { id: "day", label: "Gün", current: "Bugün", summary: "Gün" },
  { id: "week", label: "Hafta", current: "Bu hafta", summary: "Hafta" },
  { id: "month", label: "Ay", current: "Bu ay", summary: "Ay" },
];

type Props = {
  mode: Mode;
  start: string;
  title: string;
  onMode: (mode: Mode) => void;
  onPrev: () => void;
  onNext: () => void;
  onToday: (() => void) | null;
  onSelectDay: (iso: string) => void;
};

export default function ReportView(p: Props) {
  const days = p.mode === "day" ? 1 : p.mode === "week" ? 7 : daysInMonth(parseIsoDate(p.start));
  // Ay görünümünde zaman çizelgesi gerekmez (yalnızca gün toplamları).
  const timeline = p.mode !== "month";
  const [report, setReport] = useState<Report | null>(null);
  const [previous, setPrevious] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dailyHours, setDailyHours] = useState(8);
  const [limits, setLimits] = useState<CategoryLimit[]>([]);
  const [projectGoals, setProjectGoals] = useState<ProjectGoal[]>([]);
  useEffect(() => {
    api.goals().then(
      (g) => {
        setDailyHours(g.dailyHours);
        setLimits(g.limits ?? []);
        setProjectGoals(g.projectGoals ?? []);
      },
      () => {},
    );
  }, []);

  // Her yükleme bir sıra numarası alır; geç gelen eski yanıt (başka dönem) ekranı ezmez.
  const seq = useRef(0);
  const load = useCallback(() => {
    const n = ++seq.current;
    api.report(p.start, days, timeline).then(
      (r) => {
        if (n !== seq.current) return;
        setReport(r);
        setError(null);
      },
      (e) => n === seq.current && setError(String(e)),
    );
    const start = parseIsoDate(p.start);
    const prevStart = p.mode === "month" ? addMonths(start, -1) : addDays(start, -days);
    const fullPrev = p.mode === "month" ? daysInMonth(prevStart) : days;
    // Süren dönem, önceki dönemin aynı noktasına kadarki kısmıyla kıyaslanır
    // (bugün 10:00'da dünün 10:00'una kadarı; ayın 3'ünde geçen ayın ilk 3 günü).
    const elapsed = Date.now() - +start;
    const live = elapsed > 0 && elapsed < +addDays(start, days) - +start;
    const until = live ? new Date(+prevStart + elapsed).toISOString() : undefined;
    api.report(isoDate(prevStart), fullPrev, false, until).then(
      (r) => n === seq.current && setPrevious(r),
      () => n === seq.current && setPrevious(null),
    );
  }, [p.start, p.mode, days, timeline]);

  useEffect(load, [load]);
  useTauriEvent(api.onSync, load);

  const from = parseIsoDate(p.start);
  const end = addDays(from, days);
  const isLive = new Date() < end && new Date() >= from;
  // Süren dönemde takip durumu her pencere geçişinde gelir; raporu her seferinde
  // yeniden hesaplamak yerine en çok 15 sn'de bir yenile.
  const pending = useRef<number | null>(null);
  useTauriEvent(api.onStatus, () => {
    if (!isLive || pending.current !== null) return;
    pending.current = window.setTimeout(() => {
      pending.current = null;
      load();
    }, LIVE_REFRESH_MS);
  });
  useEffect(
    () => () => {
      if (pending.current !== null) window.clearTimeout(pending.current);
      pending.current = null;
    },
    [load],
  );

  const tags = useMemo(() => tagMap(report?.tags ?? []), [report]);
  const categories = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "category"), [report]);
  const projects = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "project"), [report]);
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);

  const mode = MODES.find((m) => m.id === p.mode) ?? MODES[0];
  const [draft, setDraft] = useState<EntryDraft | null>(null);
  const [calendarView, setCalendarView] = useCalendarView();
  const [preview, setPreview] = useState<[number, number] | null>(null);
  const [selection, setSelection] = useState<RangeSelection | null>(null);
  const closeSelection = useCallback(() => setSelection(null), []);
  const selectRange = useCallback(
    (start: number, end: number, x: number, y: number) => setSelection({ start, end, x, y }),
    [],
  );
  const openDraft = useCallback((start: number, end: number) => {
    const hm = (d: Date) => `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    // Form dakika hassasiyetinde: başlangıç yukarı, bitiş aşağı yuvarlanır ki komşu oturumlarla çakışmasın.
    const a = new Date(Math.ceil(start / 60_000) * 60_000);
    const b = new Date(Math.floor(end / 60_000) * 60_000);
    // Gece yarısını aşan boşluk günün sonunda kesilir (saat alanı 24:00 alamaz).
    const to = isoDate(b) === isoDate(a) ? hm(b) : "23:59";
    setDraft((d) => ({ date: isoDate(a), from: hm(a), to, seq: (d?.seq ?? 0) + 1 }));
    setPreview([start, end]);
  }, []);
  const editCtx = useMemo(() => ({ categories, projects, onChanged: load }), [categories, projects, load]);
  // Hafta görünümünde elle kayıt varsayılan olarak bugüne (haftadaysa) ya da haftanın ilk gününe.
  const todayIso = isoDate(today());
  const manualDay =
    todayIso >= p.start && todayIso < isoDate(addDays(parseIsoDate(p.start), days)) ? todayIso : p.start;

  // Yakınlaştırma: takvimde saat yüksekliği, uygulama çizelgesinde gösterilen saat aralığı.
  const appsView = p.mode !== "month" && calendarView === "apps" && !!report && report.totalSeconds > 0;
  const zoomable = p.mode !== "month" && !!report && report.totalSeconds > 0;
  const [calZoom, setCalZoom] = useState(1);
  const [appZoom, setAppZoom] = useState(1);
  const scroller = useRef<HTMLDivElement>(null);
  const [calendarArea, setCalendarArea] = useState<HTMLDivElement | null>(null);
  // Takvim yakınlaşırken imlecin altındaki saat yerinde kalsın: imlecin saat ızgarasındaki
  // konumu (son çizimdeki ölçekle) saklanır, çizimden sonra kaydırma buna göre düzeltilir.
  const rendered = useRef(1);
  const anchor = useRef<number | null>(null);
  function zoomCalendar(next: (z: number) => number, clientY?: number) {
    const grid = calendarArea?.querySelector("[data-zoom-grid]");
    const box = scroller.current?.getBoundingClientRect();
    if (grid && box && anchor.current === null) {
      const y = clientY ?? box.top + box.height / 2;
      anchor.current = Math.max(0, y - grid.getBoundingClientRect().top);
    }
    setCalZoom((z) => clampZoom(next(z)));
  }
  useLayoutEffect(() => {
    if (anchor.current !== null && scroller.current) {
      scroller.current.scrollTop += anchor.current * (calZoom / rendered.current - 1);
    }
    anchor.current = null;
    rendered.current = calZoom;
  }, [calZoom]);
  useZoomGestures(calendarArea, (factor, _x, y) => {
    // Ay görünümünde yakınlaştırma yok: gizlice değişip gün görünümüne taşınmasın.
    if (zoomable && !appsView) zoomCalendar((z) => z * factor, y);
  });
  const zoom = appsView ? appZoom : calZoom;
  // Yapışkan kart başlığının yüksekliği: takvim sütun başlıkları onun altına yapışır.
  const [head, setHead] = useState<HTMLDivElement | null>(null);
  const [headHeight, setHeadHeight] = useState(0);
  useEffect(() => {
    if (!head) return;
    const ro = new ResizeObserver(() => setHeadHeight(head.offsetHeight));
    ro.observe(head);
    return () => ro.disconnect();
  }, [head]);
  const setZoom = (next: (z: number) => number) =>
    appsView ? setAppZoom((z) => clampZoom(next(z))) : zoomCalendar(next);

  // Klavye: +/− yakınlaştır, 0 sıfırla (⌘ ile ya da tek başına).
  const zoomKeys = useRef(setZoom);
  zoomKeys.current = setZoom;
  useEffect(() => {
    if (!zoomable) return;
    function onKey(e: KeyboardEvent) {
      if (e.altKey || e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]"))
        return;
      const dir = e.key === "+" || e.key === "=" ? 1 : e.key === "-" ? -1 : e.key === "0" ? 0 : null;
      if (dir === null) return;
      e.preventDefault();
      zoomKeys.current((z) => (dir === 0 ? 1 : stepZoom(z, dir)));
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoomable]);

  return (
    <>
      <Toolbar title={p.title}>
        <Tabs value={p.mode} onValueChange={(v) => p.onMode(v as Mode)}>
          <TabsList aria-label="Görünüm">
            {MODES.map((m, i) => (
              <TabsTrigger key={m.id} value={m.id} className="px-3.5" title={`${m.label} (${i + 1})`}>
                {m.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" onClick={p.onPrev} aria-label="Önceki" title="Önceki (←)">
            <ChevronLeft />
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={p.onToday ?? undefined}
            disabled={!p.onToday}
            title="Bugüne dön (T)"
          >
            {mode.current}
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={p.onNext}
            disabled={isLive}
            aria-label="Sonraki"
            title="Sonraki (→)"
          >
            <ChevronRight />
          </Button>
        </div>
      </Toolbar>

      <div ref={scroller} className="@container flex-1 overflow-y-auto px-5 pb-6">
        {error && <p className="pb-3 text-xs text-destructive selectable">{error}</p>}
        {report && (
          <div className="grid gap-4 @[880px]:grid-cols-[minmax(0,1fr)_292px]">
            <div className="min-w-0 space-y-4">
              <Card className="gap-3 py-3" style={{ ["--cal-head" as string]: `${headHeight}px` }}>
                {(order.length > 0 || p.mode !== "month") && (
                  // Yakınlaşınca uzun takvimde başlık ve düğmeler görünür kalsın.
                  <CardContent
                    ref={setHead}
                    className="sticky top-0 z-30 -mt-3 flex items-start gap-3 rounded-t-xl bg-card pt-3 pb-1"
                  >
                    <div className="min-w-0 flex-1">
                      <Legend order={order} tags={tags} />
                    </div>
                    {p.mode !== "month" && report.totalSeconds > 0 && (
                      <Tabs value={calendarView} onValueChange={(v) => setCalendarView(v as CalendarView)}>
                        <TabsList className="h-7">
                          <TabsTrigger value="calendar" className="px-2.5 text-xs">
                            Takvim
                          </TabsTrigger>
                          <TabsTrigger value="apps" className="px-2.5 text-xs">
                            Uygulamalar
                          </TabsTrigger>
                        </TabsList>
                      </Tabs>
                    )}
                    {zoomable && <ZoomControl zoom={zoom} onZoom={setZoom} />}
                    {p.mode !== "month" && (
                      <ManualEntry
                        day={p.mode === "day" ? p.start : manualDay}
                        categories={categories}
                        projects={projects}
                        onChanged={load}
                        draft={draft}
                        onClose={() => setPreview(null)}
                      />
                    )}
                  </CardContent>
                )}
                <CardContent className="px-3">
                  {p.mode !== "month" && report.totalSeconds === 0 && <Empty future={+from > Date.now()} />}
                  <div ref={setCalendarArea}>
                    <EditContext.Provider value={editCtx}>
                      {p.mode === "month" ? (
                        <MonthCalendar
                          from={from}
                          days={report.days}
                          tags={tags}
                          dailyHours={dailyHours}
                          onSelectDay={p.onSelectDay}
                        />
                      ) : appsView ? (
                        <AppTimeline
                          from={from}
                          days={days}
                          windows={report.windows}
                          tags={tags}
                          onSelectDay={p.onSelectDay}
                          zoom={appZoom}
                          onZoom={(z) => setAppZoom(clampZoom(z))}
                          onSelectSpan={selectRange}
                        />
                      ) : p.mode === "day" ? (
                        <DayCalendar
                          from={from}
                          blocks={report.focus.blocks}
                          segments={report.timeline}
                          timers={report.focusTimers}
                          tags={tags}
                          onEmpty={openDraft}
                          onRange={selectRange}
                          preview={preview}
                          hourPx={HOUR_PX * calZoom}
                        />
                      ) : (
                        <WeekCalendar
                          from={from}
                          blocks={report.focus.blocks}
                          dayTotals={report.days.map((d) => d.seconds)}
                          timers={report.focusTimers}
                          tags={tags}
                          onSelectDay={p.onSelectDay}
                          onEmpty={openDraft}
                          onRange={selectRange}
                          preview={preview}
                          hourPx={HOUR_PX * calZoom}
                        />
                      )}
                    </EditContext.Provider>
                    {selection && (
                      <RangeMenu
                        selection={selection}
                        categories={categories}
                        projects={projects}
                        onAddEntry={openDraft}
                        onChanged={load}
                        onClose={closeSelection}
                      />
                    )}
                  </div>
                </CardContent>
              </Card>
              {report.apps.length > 0 && (
                <Card className="gap-2">
                  <CardHeader>
                    <CardTitle>Uygulamalar ve pencereler</CardTitle>
                  </CardHeader>
                  <CardContent className="px-2">
                    <AppList
                      apps={report.apps}
                      tags={tags}
                      categories={categories as Tag[]}
                      start={p.start}
                      days={days}
                      onChanged={load}
                    />
                  </CardContent>
                </Card>
              )}
            </div>
            <Summary
              report={report}
              previous={previous}
              tags={tags}
              days={days}
              dailyHours={dailyHours}
              limits={p.mode === "day" ? limits : []}
              projectGoals={p.mode === "week" ? projectGoals : []}
              mode={p.mode}
              title={isLive ? mode.current : mode.summary}
            />
          </div>
        )}
      </div>
    </>
  );
}

/** Kayıt yokken takvimin üstünde: ne olacağını ve elle eklemenin yolunu söyler. */
function Empty({ future }: { future: boolean }) {
  return (
    <div className="mx-1 mb-3 flex items-center gap-3 rounded-lg bg-muted/60 px-3 py-2.5">
      <Hourglass className="size-4 shrink-0 text-muted-foreground" />
      <div className="text-xs">
        <p className="font-medium">Bu aralık için kayıt yok</p>
        <p className="text-muted-foreground">
          {future
            ? "Kum arka planda çalışırken takvim kendiliğinden dolacak."
            : "Kum çalışırken takvim kendiliğinden dolar. Bilgisayar dışında geçen süre için boş alana tıkla."}
        </p>
      </div>
    </div>
  );
}

/** −  %100  + : yakınlaştırma düğmeleri; ortadaki değer sıfırlar. */
function ZoomControl({ zoom, onZoom }: { zoom: number; onZoom: (next: (z: number) => number) => void }) {
  return (
    <div className="flex h-7 items-center rounded-md border" role="group" aria-label="Yakınlaştırma">
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-6.5"
        onClick={() => onZoom((z) => stepZoom(z, -1))}
        disabled={zoom <= 1}
        aria-label="Uzaklaştır"
        title="Uzaklaştır (−)"
      >
        <ZoomOut />
      </Button>
      <button
        className="w-10 text-center text-[11px] text-muted-foreground tabular hover:text-foreground"
        onClick={() => onZoom(() => 1)}
        title="Sığdır (0) · ⌘ + kaydırma ya da iki parmakla da yakınlaşır"
      >
        %{Math.round(zoom * 100)}
      </button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-6.5"
        onClick={() => onZoom((z) => stepZoom(z, 1))}
        disabled={zoom >= 8}
        aria-label="Yakınlaştır"
        title="Yakınlaştır (+)"
      >
        <ZoomIn />
      </Button>
    </div>
  );
}

type CalendarView = "calendar" | "apps";
// Anahtar eski adıyla kalır: kayıtlı tercih korunsun.
const CALENDAR_VIEW_KEY = "kum.dayView";

/** Gün ve hafta görünümünde takvim mi uygulama çizelgesi mi; tercih bu cihazda hatırlanır. */
function useCalendarView(): [CalendarView, (v: CalendarView) => void] {
  const [view, setView] = useState<CalendarView>(() => {
    try {
      return localStorage.getItem(CALENDAR_VIEW_KEY) === "apps" ? "apps" : "calendar";
    } catch {
      return "calendar";
    }
  });
  return [
    view,
    (v) => {
      setView(v);
      try {
        localStorage.setItem(CALENDAR_VIEW_KEY, v);
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}
