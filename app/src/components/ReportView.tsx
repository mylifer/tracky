import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Hourglass } from "lucide-react";
import { api, type CategoryLimit, type Report, type Tag } from "../api";
import { addDays, addMonths, daysInMonth, isoDate, parseIsoDate, today } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { useTauriEvent } from "../lib/useTauriEvent";
import { AppList, Legend } from "./Breakdown";
import AppTimeline from "./AppTimeline";
import { DayCalendar, WeekCalendar } from "./Calendar";
import MonthCalendar from "./MonthCalendar";
import { EditContext, type EntryDraft, ManualEntry } from "./SessionEdit";
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
  useEffect(() => {
    api.goals().then(
      (g) => {
        setDailyHours(g.dailyHours);
        setLimits(g.limits ?? []);
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
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);

  const mode = MODES.find((m) => m.id === p.mode) ?? MODES[0];
  const [draft, setDraft] = useState<EntryDraft | null>(null);
  const [dayView, setDayView] = useDayView();
  const [preview, setPreview] = useState<[number, number] | null>(null);
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
  const editCtx = useMemo(() => ({ categories, onChanged: load }), [categories, load]);
  // Hafta görünümünde elle kayıt varsayılan olarak bugüne (haftadaysa) ya da haftanın ilk gününe.
  const todayIso = isoDate(today());
  const manualDay =
    todayIso >= p.start && todayIso < isoDate(addDays(parseIsoDate(p.start), days)) ? todayIso : p.start;

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

      <div className="@container flex-1 overflow-y-auto px-5 pb-6">
        {error && <p className="pb-3 text-xs text-destructive selectable">{error}</p>}
        {report && (
          <div className="grid gap-4 @[880px]:grid-cols-[minmax(0,1fr)_292px]">
            <div className="min-w-0 space-y-4">
              <Card className="gap-3 py-3">
                {(order.length > 0 || p.mode !== "month") && (
                  <CardContent className="flex items-start gap-3">
                    <div className="min-w-0 flex-1">
                      <Legend order={order} tags={tags} />
                    </div>
                    {p.mode !== "month" && report.totalSeconds > 0 && (
                      <Tabs value={dayView} onValueChange={(v) => setDayView(v as DayView)}>
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
                    {p.mode !== "month" && (
                      <ManualEntry
                        day={p.mode === "day" ? p.start : manualDay}
                        categories={categories}
                        onChanged={load}
                        draft={draft}
                        onClose={() => setPreview(null)}
                      />
                    )}
                  </CardContent>
                )}
                <CardContent className="px-3">
                  {p.mode !== "month" && report.totalSeconds === 0 && <Empty future={+from > Date.now()} />}
                  <EditContext.Provider value={editCtx}>
                    {p.mode === "month" ? (
                      <MonthCalendar
                        from={from}
                        days={report.days}
                        tags={tags}
                        dailyHours={dailyHours}
                        onSelectDay={p.onSelectDay}
                      />
                    ) : dayView === "apps" && report.totalSeconds > 0 ? (
                      <AppTimeline
                        from={from}
                        days={days}
                        windows={report.windows}
                        tags={tags}
                        onSelectDay={p.onSelectDay}
                      />
                    ) : p.mode === "day" ? (
                      <DayCalendar
                        from={from}
                        blocks={report.focus.blocks}
                        segments={report.timeline}
                        timers={report.focusTimers}
                        tags={tags}
                        onEmpty={openDraft}
                        preview={preview}
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
                        preview={preview}
                      />
                    )}
                  </EditContext.Provider>
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

type DayView = "calendar" | "apps";
const DAY_VIEW_KEY = "kum.dayView";

/** Gün ve hafta görünümünde takvim mi uygulama çizelgesi mi; tercih bu cihazda hatırlanır. */
function useDayView(): [DayView, (v: DayView) => void] {
  const [view, setView] = useState<DayView>(() => {
    try {
      return localStorage.getItem(DAY_VIEW_KEY) === "apps" ? "apps" : "calendar";
    } catch {
      return "calendar";
    }
  });
  return [
    view,
    (v) => {
      setView(v);
      try {
        localStorage.setItem(DAY_VIEW_KEY, v);
      } catch {
        /* depolama kapalıysa yalnızca bu oturumda */
      }
    },
  ];
}
