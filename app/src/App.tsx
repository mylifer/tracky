import { useCallback, useEffect, useState, type ReactNode } from "react";
import { CalendarDays, CalendarRange, Calendar as CalendarIcon, Download, Pause, Play, Settings2, Tags } from "lucide-react";
import { api, formatDuration, type AppStatus, type TrackingStatus } from "./api";
import ReportView from "./components/ReportView";
import Toolbar from "./components/Toolbar";
import { Button } from "./components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "./components/ui/tooltip";
import {
  addDays,
  addMonths,
  formatMonth,
  formatWeek,
  isoDate,
  parseIsoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "./lib/dates";
import { applyPlatform, useSystemTheme } from "./lib/theme";
import { useUpdate } from "./lib/useUpdate";
import { cn } from "./lib/utils";
import Onboarding from "./Onboarding";
import Categories from "./pages/Categories";
import Settings from "./pages/Settings";

type Mode = "day" | "week" | "month";
type View = Mode | "categories" | "settings";

const REPORTS: { id: Mode; label: string; icon: ReactNode }[] = [
  { id: "day", label: "Gün", icon: <CalendarDays /> },
  { id: "week", label: "Hafta", icon: <CalendarRange /> },
  { id: "month", label: "Ay", icon: <CalendarIcon /> },
];

const TITLES: Partial<Record<View, string>> = { categories: "Kategoriler ve projeler", settings: "Ayarlar" };

const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  useSystemTheme();

  const refresh = useCallback(() => {
    api.status().then(
      (s) => {
        applyPlatform(s.platform, s.effect);
        setStatus(s);
      },
      (e) => setError(String(e)),
    );
  }, []);

  useEffect(refresh, [refresh]);

  if (error)
    return (
      <main className="grid h-full place-items-center bg-background p-6">
        <p className="text-destructive selectable">{error}</p>
      </main>
    );
  if (!status) return null;
  if (!status.onboarded || !status.accessibility) return <Onboarding status={status} onChange={refresh} />;
  return <Shell status={status} refresh={refresh} />;
}

function Shell({ status, refresh }: { status: AppStatus; refresh: () => void }) {
  const [view, setView] = useState<View>("day");
  const [day, setDay] = useState(isoDate(today()));
  const [week, setWeek] = useState(isoDate(startOfWeek(today())));
  const [month, setMonth] = useState(isoDate(startOfMonth(today())));
  const [tracking, setTracking] = useState<TrackingStatus>(status.tracking);
  const [update] = useUpdate();
  const isMac = status.platform === "macos";

  useEffect(() => {
    const unlisten = api.onStatus(setTracking);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function togglePause() {
    await api.setPaused(!tracking.paused);
    setTracking({ ...tracking, paused: !tracking.paused });
  }

  const shift = (iso: string, n: number) => isoDate(addDays(parseIsoDate(iso), n));
  const todayIso = isoDate(today());
  const thisWeek = isoDate(startOfWeek(today()));
  const thisMonth = isoDate(startOfMonth(today()));
  const dayDate = parseIsoDate(day);

  function step(n: number) {
    if (view === "day") setDay(shift(day, n));
    else if (view === "week") setWeek(shift(week, 7 * n));
    else setMonth(isoDate(addMonths(parseIsoDate(month), n)));
  }

  function selectMode(m: Mode) {
    if (m === "week") setWeek(isoDate(startOfWeek(dayDate)));
    if (m === "month") setMonth(isoDate(startOfMonth(view === "week" ? parseIsoDate(week) : dayDate)));
    setView(m);
  }

  return (
    <div className="flex h-full">
      <aside className="flex w-[216px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground material:bg-transparent">
        {/* macOS'ta pencere düğmeleri bu şeridin üstünde durur. */}
        <div data-tauri-drag-region className="flex h-[52px] shrink-0 items-center gap-2 px-4">
          {!isMac && (
            <>
              <img src="/icon.png" alt="" className="pointer-events-none size-5" />
              <span className="pointer-events-none text-[13px] font-semibold">Kum</span>
            </>
          )}
        </div>
        <nav className="flex flex-1 flex-col gap-5 overflow-y-auto px-2.5 pt-1">
          <NavSection title="Raporlar">
            {REPORTS.map((r) => (
              <NavItem key={r.id} icon={r.icon} active={view === r.id} onClick={() => selectMode(r.id)}>
                {r.label}
              </NavItem>
            ))}
          </NavSection>
          <NavSection title="Düzenle">
            <NavItem icon={<Tags />} active={view === "categories"} onClick={() => setView("categories")}>
              Kategoriler
            </NavItem>
            <NavItem icon={<Settings2 />} active={view === "settings"} onClick={() => setView("settings")}>
              Ayarlar
            </NavItem>
          </NavSection>
        </nav>
        <div className="space-y-2 p-2.5">
          {update?.ready && (
            <button
              onClick={() => setView("settings")}
              className="flex w-full items-center gap-2.5 rounded-lg bg-primary/12 px-2.5 py-2 text-left text-xs transition-colors hover:bg-primary/18"
            >
              <Download className="size-4 shrink-0 text-primary" />
              <span className="min-w-0">
                <span className="block font-medium">Güncelleme hazır</span>
                <span className="block text-muted-foreground">Kum {update.available} · yüklemek için tıkla</span>
              </span>
            </button>
          )}
          <LiveCard tracking={tracking} onToggle={togglePause} />
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col bg-background mica:bg-background/75">
        {view === "day" || view === "week" || view === "month" ? (
          <ReportView
            mode={view}
            start={view === "day" ? day : view === "week" ? week : month}
            title={
              view === "day"
                ? capitalize(longDate.format(dayDate))
                : view === "week"
                  ? formatWeek(parseIsoDate(week))
                  : capitalize(formatMonth(parseIsoDate(month)))
            }
            onMode={selectMode}
            onPrev={() => step(-1)}
            onNext={() => step(1)}
            onToday={
              view === "day"
                ? day === todayIso
                  ? null
                  : () => setDay(todayIso)
                : view === "week"
                  ? week === thisWeek
                    ? null
                    : () => setWeek(thisWeek)
                  : month === thisMonth
                    ? null
                    : () => setMonth(thisMonth)
            }
            onSelectDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
          />
        ) : (
          <>
            <Toolbar title={TITLES[view] ?? ""} />
            <div className="flex-1 overflow-y-auto">
              {view === "categories" && <Categories />}
              {view === "settings" && <Settings status={status} onChange={refresh} />}
            </div>
          </>
        )}
      </main>
    </div>
  );
}

function NavSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="space-y-0.5">
      <div className="px-2 pb-1 text-[11px] font-semibold text-muted-foreground">{title}</div>
      {children}
    </div>
  );
}

function NavItem({
  icon,
  active,
  onClick,
  children,
}: {
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-7 w-full items-center gap-2 rounded-md px-2 text-[13px] transition-colors [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-primary",
        active ? "bg-sidebar-accent font-medium" : "hover:bg-sidebar-accent/50",
      )}
    >
      {icon}
      {children}
    </button>
  );
}

/** Kenar çubuğunun altında: şu an ne takip ediliyor, bugünkü toplam, duraklat. */
function LiveCard({ tracking, onToggle }: { tracking: TrackingStatus; onToggle: () => void }) {
  const state = tracking.paused
    ? "Duraklatıldı"
    : tracking.needsPermission
      ? "İzin gerekli"
      : tracking.current
        ? tracking.current.appName
        : "Boşta";
  const live = !tracking.paused && !tracking.needsPermission && !!tracking.current;
  return (
    <div className="rounded-lg border border-sidebar-border bg-background/70 p-2.5 shadow-xs dark:bg-white/5">
      <div className="flex items-start gap-2">
        <span className="relative mt-[5px] flex size-2 shrink-0">
          {live && <span className="absolute inline-flex size-full animate-ping rounded-full bg-success opacity-60" />}
          <span className={cn("relative inline-flex size-2 rounded-full", live ? "bg-success" : "bg-muted-foreground/50")} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium">{state}</div>
          {live && tracking.current?.title && (
            <div className="truncate text-[11px] text-muted-foreground" title={tracking.current.title}>
              {tracking.current.title}
            </div>
          )}
        </div>
      </div>
      <div className="mt-2 flex items-end justify-between">
        <div>
          <div className="text-[11px] text-muted-foreground">Bugün</div>
          <div className="text-[15px] font-semibold tabular">{formatDuration(tracking.todaySeconds)}</div>
        </div>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="icon-sm"
              className="rounded-full"
              onClick={onToggle}
              aria-label={tracking.paused ? "Devam et" : "Duraklat"}
            >
              {tracking.paused ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{tracking.paused ? "Takibe devam et" : "Takibi duraklat"}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
