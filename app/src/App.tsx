import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  CalendarDays,
  CalendarRange,
  Calendar as CalendarIcon,
  Pause,
  Play,
  Search as SearchIcon,
  TrendingUp,
  Settings2,
  Tags,
} from "lucide-react";
import { api, formatDuration, type AppStatus, type TrackingStatus } from "./api";
import FocusCard from "./components/FocusCard";
import { UpdateCard } from "./components/UpdateCard";
import ReportView from "./components/ReportView";
import Toolbar from "./components/Toolbar";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "./components/ui/tooltip";
import {
  addDays,
  addMonths,
  formatMonth,
  formatTime,
  formatWeek,
  isoDate,
  parseIsoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "./lib/dates";
import { applyPlatform, useTheme } from "./lib/theme";
import { useUpdate } from "./lib/useUpdate";
import { cn } from "./lib/utils";
import Onboarding from "./Onboarding";
import Categories from "./pages/Categories";
import Search, { type SearchState } from "./pages/Search";
import Trends from "./pages/Trends";
import Settings from "./pages/Settings";
import { useTauriEvent } from "./lib/useTauriEvent";

type Mode = "day" | "week" | "month";
type View = Mode | "trends" | "search" | "categories" | "settings";

const REPORTS: { id: Mode; label: string; icon: ReactNode }[] = [
  { id: "day", label: "Gün", icon: <CalendarDays /> },
  { id: "week", label: "Hafta", icon: <CalendarRange /> },
  { id: "month", label: "Ay", icon: <CalendarIcon /> },
];

const TITLES: Partial<Record<View, string>> = {
  trends: "Eğilimler",
  search: "Ara",
  categories: "Kategoriler ve projeler",
  settings: "Ayarlar",
};

const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  useTheme(status?.theme ?? "system");

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
  // Arama görünümden çıkınca kaybolmasın.
  const [search, setSearch] = useState<SearchState>({ query: "", days: 30 });
  // Bekleyen öneri sayısı (kenar çubuğunda); görünüm değişince ve saatte bir yenilenir.
  const [suggestionCount, setSuggestionCount] = useState(0);
  useEffect(() => {
    const refresh = () =>
      api.suggestions().then(
        (s) => setSuggestionCount(s.projects.length + s.categories.length),
        () => {},
      );
    refresh();
    const id = setInterval(refresh, 3600_000);
    return () => clearInterval(id);
  }, [view]);
  const [day, setDay] = useState(isoDate(today()));
  const [week, setWeek] = useState(isoDate(startOfWeek(today())));
  const [month, setMonth] = useState(isoDate(startOfMonth(today())));
  const [tracking, setTracking] = useState<TrackingStatus>(status.tracking);
  const [update, setUpdate] = useUpdate();
  const isMac = status.platform === "macos";

  useTauriEvent(api.onStatus, setTracking);

  async function togglePause() {
    const paused = !tracking.paused;
    try {
      await api.setPaused(paused);
      // Beklerken gelen durum güncellemesini ezmemek için güncel değerin üzerine yaz.
      setTracking((t) => ({ ...t, paused }));
    } catch {
      /* durum olayı doğru değeri yeniden getirir */
    }
  }

  const shift = (iso: string, n: number) => isoDate(addDays(parseIsoDate(iso), n));
  const todayIso = isoDate(today());
  const thisWeek = isoDate(startOfWeek(today()));
  const thisMonth = isoDate(startOfMonth(today()));
  const dayDate = parseIsoDate(day);

  // Seçili dönem bugünü içeriyor mu? (İleri gidilemez, "Bugün" düğmesi pasif.)
  const atCurrent =
    view === "day"
      ? day === todayIso
      : view === "week"
        ? week === thisWeek
        : view === "month"
          ? month === thisMonth
          : true;

  function step(n: number) {
    if (n > 0 && atCurrent) return;
    if (view === "day") setDay(shift(day, n));
    else if (view === "week") setWeek(shift(week, 7 * n));
    else if (view === "month") setMonth(isoDate(addMonths(parseIsoDate(month), n)));
  }

  function goToday() {
    if (view === "day") setDay(todayIso);
    else if (view === "week") setWeek(thisWeek);
    else if (view === "month") setMonth(thisMonth);
  }

  // Klavye: ←/→ önceki/sonraki dönem, T bugün, 1/2/3 Gün/Hafta/Ay, / arama.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]"))
        return;
      const report = view === "day" || view === "week" || view === "month";
      if (e.key === "1") selectMode("day");
      else if (e.key === "2") selectMode("week");
      else if (e.key === "3") selectMode("month");
      else if (report && e.key === "ArrowLeft") step(-1);
      else if (report && e.key === "ArrowRight") step(1);
      else if (report && (e.key === "t" || e.key === "T")) goToday();
      else if (e.key === "/") setView("search");
      else return;
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function selectMode(m: Mode) {
    if (m === "week") setWeek(isoDate(startOfWeek(dayDate)));
    if (m === "month") {
      // Bu hafta seçiliyse bugünün ayı (hafta önceki aydan başlasa bile).
      const ref = view === "week" ? (week === thisWeek ? today() : parseIsoDate(week)) : dayDate;
      setMonth(isoDate(startOfMonth(ref)));
    }
    setView(m);
  }

  return (
    <div className="flex h-full">
      <aside className="flex w-[216px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground material:bg-transparent">
        {/* macOS'ta pencere düğmeleri bu şeridin üstünde durur; Windows'ta yerel başlık
            çubuğu ad ve simgeyi zaten gösterir, burada tekrarlanmaz. */}
        <div data-tauri-drag-region className={cn("shrink-0", isMac ? "h-[52px]" : "h-3.5")} />
        <nav className="flex flex-1 flex-col gap-5 overflow-y-auto px-2.5 pt-1">
          <NavSection title="Raporlar">
            {REPORTS.map((r) => (
              <NavItem key={r.id} icon={r.icon} active={view === r.id} onClick={() => selectMode(r.id)}>
                {r.label}
              </NavItem>
            ))}
            <NavItem icon={<TrendingUp />} active={view === "trends"} onClick={() => setView("trends")}>
              Eğilimler
            </NavItem>
            <NavItem icon={<SearchIcon />} active={view === "search"} onClick={() => setView("search")}>
              Ara
            </NavItem>
          </NavSection>
          <NavSection title="Düzenle">
            <NavItem
              icon={<Tags />}
              active={view === "categories"}
              onClick={() => setView("categories")}
              badge={suggestionCount}
            >
              Kategoriler
            </NavItem>
            <NavItem icon={<Settings2 />} active={view === "settings"} onClick={() => setView("settings")}>
              Ayarlar
            </NavItem>
          </NavSection>
        </nav>
        <div className="space-y-2 p-2.5">
          <UpdateCard status={update} onStatus={setUpdate} />
          <FocusCard focus={tracking.focus} />
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
            onToday={atCurrent ? null : goToday}
            onSelectDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
          />
        ) : (
          <>
            <Toolbar title={TITLES[view] ?? ""} />
            <div className="flex-1 overflow-y-auto">
              {view === "trends" && (
                <Trends
                  onSearch={(query) => {
                    setSearch({ query, days: 30 });
                    setView("search");
                  }}
                />
              )}
              {view === "search" && (
                <Search
                  state={search}
                  onChange={setSearch}
                  onSelectDay={(iso) => {
                    setDay(iso);
                    setView("day");
                  }}
                />
              )}
              {view === "categories" && <Categories onSuggestions={setSuggestionCount} />}
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
  badge,
  children,
}: {
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  /** Sağda küçük sayı (örn. bekleyen öneriler). */
  badge?: number;
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
      {!!badge && (
        <span className="ml-auto rounded-full bg-primary/15 px-1.5 text-[10px] leading-4 font-semibold text-primary tabular">
          {badge}
        </span>
      )}
    </button>
  );
}

/** Kenar çubuğunun altında: şu an ne takip ediliyor, bugünkü toplam, duraklat. */
function LiveCard({ tracking, onToggle }: { tracking: TrackingStatus; onToggle: () => void }) {
  const [menu, setMenu] = useState(false);
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
          <span
            className={cn("relative inline-flex size-2 rounded-full", live ? "bg-success" : "bg-muted-foreground/50")}
          />
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium">{state}</div>
          {tracking.paused && tracking.pausedUntil && (
            <div className="text-[11px] text-muted-foreground tabular">
              Devam: {formatTime(new Date(tracking.pausedUntil))}
            </div>
          )}
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
        {tracking.paused ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="outline"
                size="icon-sm"
                className="rounded-full"
                onClick={onToggle}
                aria-label="Devam et"
              >
                <Play className="size-3.5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Takibe devam et</TooltipContent>
          </Tooltip>
        ) : (
          <Popover open={menu} onOpenChange={setMenu}>
            <PopoverTrigger asChild>
              <Button variant="outline" size="icon-sm" className="rounded-full" aria-label="Duraklat" title="Duraklat">
                <Pause className="size-3.5" />
              </Button>
            </PopoverTrigger>
            <PopoverContent side="top" align="end" className="w-44 p-1">
              {PAUSE_OPTIONS.map(([label, minutes]) => (
                <button
                  key={label}
                  className="flex h-7 w-full items-center rounded-md px-2 text-left text-xs hover:bg-accent"
                  onClick={() => {
                    setMenu(false);
                    if (minutes === "forever") onToggle();
                    else api.pauseFor(minutes).catch(() => {});
                  }}
                >
                  {label}
                </button>
              ))}
            </PopoverContent>
          </Popover>
        )}
      </div>
    </div>
  );
}

/** Duraklatma seçenekleri: dakika, `null` yarına kadar, "forever" süresiz. */
const PAUSE_OPTIONS: [string, number | null | "forever"][] = [
  ["15 dakika", 15],
  ["1 saat", 60],
  ["Yarına kadar", null],
  ["Ben devam ettirene kadar", "forever"],
];

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
