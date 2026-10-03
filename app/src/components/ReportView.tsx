import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Hourglass } from "lucide-react";
import { api, type CategoryLimit, type Report, type Tag } from "../api";
import { addDays, addMonths, daysInMonth, isoDate, parseIsoDate, today } from "../lib/dates";
import { tagMap } from "../lib/tags";
import { AppList, Legend } from "./Breakdown";
import { DayCalendar, WeekCalendar } from "./Calendar";
import MonthCalendar from "./MonthCalendar";
import { EditContext, ManualEntry } from "./SessionEdit";
import Summary from "./Summary";
import Toolbar from "./Toolbar";
import { Button } from "./ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "./ui/card";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

export type Mode = "day" | "week" | "month";

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

  const load = useCallback(() => {
    api.report(p.start, days, timeline).then(
      (r) => {
        setReport(r);
        setError(null);
      },
      (e) => setError(String(e)),
    );
    const start = parseIsoDate(p.start);
    const prevStart = p.mode === "month" ? addMonths(start, -1) : addDays(start, -days);
    // Süren dönem önceki dönemin aynı uzunluktaki başıyla kıyaslanır
    // (ayın 3'ünde geçen ayın tamamıyla değil, ilk 3 günüyle).
    const elapsed = Math.round((+today() - +start) / 86_400_000) + 1;
    const fullPrev = p.mode === "month" ? daysInMonth(prevStart) : days;
    const prevDays = elapsed > 0 && elapsed < days ? Math.min(elapsed, fullPrev) : fullPrev;
    api.report(isoDate(prevStart), prevDays, false).then(setPrevious, () => setPrevious(null));
  }, [p.start, p.mode, days, timeline]);

  useEffect(load, [load]);
  useEffect(() => {
    const unlisten = api.onSync(load);
    return () => {
      unlisten.then((f) => f());
    };
  }, [load]);

  const from = parseIsoDate(p.start);
  const end = addDays(from, days);
  const isLive = new Date() < end && new Date() >= from;
  useEffect(() => {
    if (!isLive) return;
    const unlisten = api.onStatus(() => api.report(p.start, days, timeline).then(setReport));
    return () => {
      unlisten.then((f) => f());
    };
  }, [isLive, p.start, days, timeline]);

  const tags = useMemo(() => tagMap(report?.tags ?? []), [report]);
  const categories = useMemo(() => (report?.tags ?? []).filter((t) => t.kind === "category"), [report]);
  const order = useMemo(() => {
    const present = new Set((report?.categories ?? []).map((c) => c.id));
    const ids: (string | null)[] = categories.map((c) => c.id).filter((id) => present.has(id));
    if (present.has(null)) ids.push(null);
    return ids;
  }, [report, categories]);

  const mode = MODES.find((m) => m.id === p.mode) ?? MODES[0];
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
            {MODES.map((m) => (
              <TabsTrigger key={m.id} value={m.id} className="px-3.5">
                {m.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" onClick={p.onPrev} aria-label="Önceki">
            <ChevronLeft />
          </Button>
          <Button variant="outline" size="sm" onClick={p.onToday ?? undefined} disabled={!p.onToday}>
            {mode.current}
          </Button>
          <Button variant="ghost" size="icon-sm" onClick={p.onNext} disabled={isLive} aria-label="Sonraki">
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
                    {p.mode !== "month" && (
                      <ManualEntry
                        day={p.mode === "day" ? p.start : manualDay}
                        categories={categories}
                        onChanged={load}
                      />
                    )}
                  </CardContent>
                )}
                <CardContent className="px-3">
                  <EditContext.Provider value={editCtx}>
                    {p.mode === "month" ? (
                      <MonthCalendar
                        from={from}
                        days={report.days}
                        tags={tags}
                        dailyHours={dailyHours}
                        onSelectDay={p.onSelectDay}
                      />
                    ) : report.totalSeconds === 0 ? (
                      <Empty />
                    ) : p.mode === "day" ? (
                      <DayCalendar from={from} blocks={report.focus.blocks} segments={report.timeline} tags={tags} />
                    ) : (
                      <WeekCalendar
                        from={from}
                        blocks={report.focus.blocks}
                        dayTotals={report.days.map((d) => d.seconds)}
                        tags={tags}
                        onSelectDay={p.onSelectDay}
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

function Empty() {
  return (
    <div className="flex flex-col items-center gap-1.5 py-16 text-center">
      <div className="mb-2 grid size-11 place-items-center rounded-full bg-muted">
        <Hourglass className="size-5 text-muted-foreground" />
      </div>
      <p className="text-[13px] font-medium">Bu aralık için kayıt yok</p>
      <p className="text-xs text-muted-foreground">Kum arka planda çalışırken takvim kendiliğinden dolacak.</p>
    </div>
  );
}
