import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  CalendarDays,
  CalendarRange,
  Calendar as CalendarIcon,
  Pause,
  Play,
  FileSpreadsheet,
  Search as SearchIcon,
  TrendingUp,
  Settings2,
  Tags,
  FolderKanban,
  Building2,
  Inbox,
  RotateCw,
  CalendarCheck,
  Moon,
  Sun,
  Monitor,
  RefreshCw,
  HardDriveDownload,
  Download,
  Command as CommandIcon,
  ReceiptText,
} from "lucide-react";
import { api, formatDuration, type AppStatus, type Suggestions, type Tag, type TrackingStatus } from "./api";
import { UpdateCard } from "./components/UpdateCard";
import ReportView from "./components/ReportView";
import Toolbar from "./components/Toolbar";
import { Toaster } from "./components/Toaster";
import { CommandPalette, type Command } from "./components/CommandPalette";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "./components/ui/tooltip";
import {
  addDays,
  addMonths,
  daysInMonth,
  formatMonth,
  formatTime,
  formatWeek,
  isoDate,
  parseIsoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "./lib/dates";
import { friendlyError, toast, useChanged } from "./lib/feedback";
import { activeProjects } from "./lib/tags";
import { applyPlatform, useTheme, type ThemePref } from "./lib/theme";
import { useUpdate } from "./lib/useUpdate";
import { cn } from "./lib/utils";
import Onboarding from "./Onboarding";
import TagsPage from "./pages/TagsPage";
import ClientsPage from "./pages/ClientsPage";
import Search, { type SearchState } from "./pages/Search";
import Trends from "./pages/Trends";
import ClientReport from "./pages/ClientReport";
import Timesheet from "./pages/Timesheet";
import Review, { type ReviewRange } from "./pages/Review";
import Settings from "./pages/Settings";
import { useTauriEvent } from "./lib/useTauriEvent";

type Mode = "day" | "week" | "month";
type View =
  | Mode
  | "review"
  | "timesheet"
  | "trends"
  | "client-report"
  | "search"
  | "clients"
  | "projects"
  | "categories"
  | "settings";

const REPORTS: { id: Mode; label: string; icon: ReactNode }[] = [
  { id: "day", label: "Gün", icon: <CalendarDays /> },
  { id: "week", label: "Hafta", icon: <CalendarRange /> },
  { id: "month", label: "Ay", icon: <CalendarIcon /> },
];

const TITLES: Partial<Record<View, string>> = {
  review: "Gözden geçir",
  timesheet: "Zaman çizelgesi",
  trends: "Eğilimler",
  "client-report": "Müşteri raporu",
  search: "Ara",
  clients: "Müşteriler",
  projects: "Projeler",
  categories: "Kategoriler",
  settings: "Ayarlar",
};

/** Mac'te ⌘, Windows'ta Ctrl ile kısayollar; ipuçlarında da böyle yazılır. */
const isMacUA = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
const MOD = isMacUA ? "⌘" : "Ctrl+";

const longDate = new Intl.DateTimeFormat("tr-TR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  useTheme(status?.theme ?? "system");

  const refresh = useCallback(() => {
    setError(null);
    api.status().then(
      (s) => {
        applyPlatform(s.platform, s.effect);
        setStatus(s);
      },
      (e) => setError(friendlyError(e)),
    );
  }, []);

  useEffect(refresh, [refresh]);

  if (error)
    return (
      <main data-tauri-drag-region className="grid h-full place-items-center bg-background p-6">
        <div className="max-w-sm space-y-4 text-center">
          <img src="/icon.png" alt="" className="mx-auto size-12 opacity-80" />
          <div>
            <p className="text-[15px] font-semibold">Kum açılamadı</p>
            <p className="mt-1 text-[13px] text-muted-foreground selectable">{error}</p>
          </div>
          <Button onClick={refresh}>
            <RotateCw /> Tekrar dene
          </Button>
        </div>
      </main>
    );
  if (!status) return <Splash />;
  if (!status.onboarded || !status.accessibility)
    return (
      <>
        <Onboarding status={status} onChange={refresh} />
        {/* Karşılamadaki hatalar da bildirimle görünsün. */}
        <Toaster />
      </>
    );
  return <Shell status={status} refresh={refresh} />;
}

/** Açılırken (durum gelene kadar) boş pencere yerine. */
function Splash() {
  return (
    <main data-tauri-drag-region className="grid h-full place-items-center bg-background">
      <img src="/icon.png" alt="" className="size-14 animate-pulse" />
    </main>
  );
}

function Shell({ status, refresh }: { status: AppStatus; refresh: () => void }) {
  const [view, setView] = useState<View>("day");
  // Gözden geçir bir aralıkla açılabilir (raporda bakılan dönem); kenar çubuğundan açılınca kendi dönemi.
  const [reviewRange, setReviewRange] = useState<ReviewRange | null>(null);
  const openReview = (range: ReviewRange | null = null) => {
    setReviewRange(range);
    setView("review");
  };
  useEffect(() => {
    if (view !== "review") setReviewRange(null);
  }, [view]);
  // Ayarlar başka bir sayfadan belirli bir bölümle açılabilir (zaman çizelgesi → Bağlantılar).
  const [settingsSection, setSettingsSection] = useState<string | null>(null);
  const openSettings = (section: string | null = null) => {
    setSettingsSection(section);
    setView("settings");
  };
  // Arama görünümden çıkınca kaybolmasın.
  const [search, setSearch] = useState<SearchState>({ query: "", days: 30 });
  // Bekleyen öneri sayıları (kenar çubuğunda); görünüm değişince ve saatte bir yenilenir.
  const [suggestionCount, setSuggestionCount] = useState({ projects: 0, categories: 0 });
  const onSuggestions = useCallback(
    (s: Suggestions) => setSuggestionCount({ projects: s.projects.length, categories: s.categories.length }),
    [],
  );
  // Kenar çubuğu rozetleri: bu haftanın atanmamış süresi ve aktarılmamış günleri.
  const [unassigned, setUnassigned] = useState(0);
  const [pendingDays, setPendingDays] = useState(0);
  const refreshBadges = useCallback(() => {
    api.suggestions().then(onSuggestions, () => {});
    api.unassigned(isoDate(startOfWeek(today())), 7).then(
      (u) => setUnassigned(u.totalSeconds),
      () => {},
    );
    api.pendingTimesheetDays().then(
      (d) => setPendingDays(d.length),
      () => {},
    );
  }, [onSuggestions]);
  useEffect(() => {
    refreshBadges();
    const id = setInterval(refreshBadges, 3600_000);
    return () => clearInterval(id);
  }, [view, refreshBadges]);
  useChanged(refreshBadges);

  const [day, setDay] = useState(isoDate(today()));
  const [week, setWeek] = useState(isoDate(startOfWeek(today())));
  const [month, setMonth] = useState(isoDate(startOfMonth(today())));
  const [tracking, setTracking] = useState<TrackingStatus>(status.tracking);
  const [update, setUpdate] = useUpdate();
  const [palette, setPalette] = useState(false);
  const [dailyHours, setDailyHours] = useState(8);
  useEffect(() => {
    api.goals().then(
      (g) => setDailyHours(g.dailyHours),
      () => {},
    );
  }, [view]);
  const [projects, setProjects] = useState<Tag[]>([]);
  useEffect(() => {
    if (palette)
      api.taxonomy().then(
        (t) => setProjects(activeProjects(t.tags)),
        () => {},
      );
  }, [palette]);
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

  // Seçili dönem bugünü içeriyor mu? ("Bugün" düğmesi pasif.)
  const atCurrent =
    view === "day"
      ? day === todayIso
      : view === "week"
        ? week === thisWeek
        : view === "month"
          ? month === thisMonth
          : true;

  // Gelecek de gezilebilir: takvimdeki yaklaşan toplantılar görünsün.
  function step(n: number) {
    if (view === "day") setDay(shift(day, n));
    else if (view === "week") setWeek(shift(week, 7 * n));
    else if (view === "month") setMonth(isoDate(addMonths(parseIsoDate(month), n)));
  }

  function goToday() {
    if (view === "day" || view === "week" || view === "month") {
      setDay(todayIso);
      setWeek(thisWeek);
      setMonth(thisMonth);
    } else {
      setDay(todayIso);
      setView("day");
    }
  }

  function selectMode(m: Mode) {
    if (m === "week") setWeek(isoDate(startOfWeek(dayDate)));
    if (m === "month") {
      // Bu hafta seçiliyse bugünün ayı (hafta önceki aydan başlasa bile).
      const ref = view === "week" ? (week === thisWeek ? today() : parseIsoDate(week)) : dayDate;
      setMonth(isoDate(startOfMonth(ref)));
    }
    setView(m);
  }

  /** Menüden, menü çubuğundan, bildirimden ya da kısayoldan gelen sayfa isteği. */
  function navigate(target: string) {
    switch (target) {
      case "palette":
        return setPalette(true);
      case "day":
      case "week":
      case "month":
        return selectMode(target);
      case "today":
        return goToday();
      case "last-week":
        setWeek(isoDate(addDays(startOfWeek(today()), -7)));
        return setView("week");
      case "settings":
        return openSettings();
      case "review":
      case "timesheet":
      case "search":
      case "trends":
      case "client-report":
      case "projects":
      case "categories":
      case "clients":
        return setView(target);
    }
  }
  useTauriEvent(api.onNavigate, navigate);

  // Klavye: ←/→ önceki/sonraki dönem, T bugün, 1/2/3 Gün/Hafta/Ay, / arama;
  // ⌘K palet, ⌘F arama, ⌘, ayarlar, ⌘1…5 sayfalar (Mac'te menü de aynı isteği gönderir).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.defaultPrevented || e.altKey) return;
      const mod = e.metaKey || e.ctrlKey;
      if (mod) {
        const target = (
          {
            k: "palette",
            f: "search",
            ",": "settings",
            "1": "day",
            "2": "week",
            "3": "month",
            "4": "timesheet",
            "5": "review",
            t: "today",
          } as Record<string, string>
        )[e.key.toLowerCase()];
        if (!target || e.shiftKey) return;
        e.preventDefault();
        navigate(target);
        return;
      }
      const el = e.target as HTMLElement | null;
      if (el?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]")) return;
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

  async function changeTheme(t: ThemePref) {
    await api.setTheme(t).catch(() => {});
    refresh();
  }

  const commands = useMemo<Command[]>(() => {
    const go = (id: View, label: string, icon: ReactNode, hint?: string, keywords?: string): Command => ({
      id: `go:${id}`,
      label,
      group: "Git",
      icon,
      hint,
      keywords,
      run: () => (id === "day" || id === "week" || id === "month" ? selectMode(id) : setView(id)),
    });
    const act = (id: string, label: string, icon: ReactNode, run: () => void, keywords?: string): Command => ({
      id,
      label,
      group: "Eylemler",
      icon,
      keywords,
      run,
    });
    const done = (p: Promise<unknown>, ok: string) =>
      p.then(
        () => toast(ok, { tone: "success" }),
        (e) => toast(friendlyError(e), { tone: "error" }),
      );
    return [
      go("day", "Gün", <CalendarDays />, `${MOD}1`, "bugün rapor takvim"),
      go("week", "Hafta", <CalendarRange />, `${MOD}2`, "haftalık rapor"),
      go("month", "Ay", <CalendarIcon />, `${MOD}3`, "aylık rapor"),
      go("timesheet", "Zaman çizelgesi", <FileSpreadsheet />, `${MOD}4`, "excel sheets aktar timesheet"),
      go("review", "Gözden geçir", <Inbox />, `${MOD}5`, "atanmamış projesiz süre"),
      go("trends", "Eğilimler", <TrendingUp />, undefined, "grafik hafta"),
      go("client-report", "Müşteri raporu", <ReceiptText />, undefined, "aylık fatura onay excel pdf yazdır"),
      go("search", "Ara", <SearchIcon />, `${MOD}F`, "bul pencere başlık"),
      go("clients", "Müşteriler", <Building2 />, undefined, "firma"),
      go("projects", "Projeler", <FolderKanban />, undefined, "kural"),
      go("categories", "Kategoriler", <Tags />, undefined, "kural uygulama"),
      go("settings", "Ayarlar", <Settings2 />, `${MOD},`, "tercihler"),
      act("today", "Bugüne git", <CalendarCheck />, goToday, "today"),
      ...(tracking.paused
        ? [act("resume", "Takibe devam et", <Play />, togglePause, "başlat")]
        : [
            act("pause-15", "15 dakika duraklat", <Pause />, () => api.pauseFor(15).catch(() => {}), "dur mola"),
            act("pause-60", "1 saat duraklat", <Pause />, () => api.pauseFor(60).catch(() => {}), "dur mola"),
            act("pause-tomorrow", "Yarına kadar duraklat", <Pause />, () => api.pauseFor(null).catch(() => {})),
          ]),
      act("theme-system", "Görünüm: Sistem", <Monitor />, () => changeTheme("system"), "tema"),
      act("theme-light", "Görünüm: Açık", <Sun />, () => changeTheme("light"), "tema aydınlık"),
      act("theme-dark", "Görünüm: Koyu", <Moon />, () => changeTheme("dark"), "tema karanlık gece"),
      act("sync", "Şimdi eşitle", <RefreshCw />, () => done(api.syncNow(), "Eşitleme başladı"), "senkron supabase"),
      act("backup", "Şimdi yedekle", <HardDriveDownload />, () => done(api.backupNow(), "Yedek alındı"), "yedek"),
      act("update", "Güncellemeleri denetle", <Download />, () => done(api.checkUpdate(), "Denetlendi"), "sürüm"),
      ...projects.map<Command>((p) => ({
        id: `project:${p.id}`,
        label: p.name,
        group: "Projeler",
        icon: <i className="mx-0.5 block size-2.5 rounded-full" style={{ background: `var(--c${p.color})` }} />,
        keywords: "proje",
        run: () => {
          setSearch({ query: p.name, days: 30 });
          setView("search");
        },
      })),
    ];
    // Palet açıkken güncel durum yeterli; her çizimde yeniden kurmak ucuz.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tracking.paused, projects, day, week, month, view]);

  const title =
    view === "day"
      ? capitalize(longDate.format(dayDate))
      : view === "week"
        ? formatWeek(parseIsoDate(week))
        : capitalize(formatMonth(parseIsoDate(month)));

  return (
    <div className="flex h-full">
      <aside className="flex w-[216px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground material:bg-transparent">
        {/* macOS'ta pencere düğmeleri bu şeridin üstünde durur; Windows'ta yerel başlık
            çubuğu ad ve simgeyi zaten gösterir, burada tekrarlanmaz. */}
        <div data-tauri-drag-region className={cn("shrink-0", isMac ? "h-[52px]" : "h-3.5")} />
        <button
          onClick={() => setPalette(true)}
          className="mx-2.5 mb-3 flex h-7 items-center gap-2 rounded-md border border-sidebar-border bg-background/60 px-2 text-xs text-muted-foreground shadow-xs transition-colors outline-none hover:bg-background hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-white/5 dark:hover:bg-white/10"
        >
          <SearchIcon className="size-3.5" />
          <span className="flex-1 text-left">Ara ya da git…</span>
          <kbd className="flex items-center gap-0.5 rounded bg-muted px-1 text-[10px] font-medium">
            {isMac ? <CommandIcon className="size-2.5" /> : "Ctrl+"}K
          </kbd>
        </button>
        <nav className="flex flex-1 flex-col gap-5 overflow-y-auto px-2.5 pt-1">
          <NavSection title="İş">
            <NavItem
              icon={<Inbox />}
              active={view === "review"}
              onClick={() => setView("review")}
              badge={unassigned >= 60 ? shortDuration(unassigned) : undefined}
              badgeTitle="Bu hafta projeye atanmamış süre"
              tone="brand"
            >
              Gözden geçir
            </NavItem>
            <NavItem
              icon={<FileSpreadsheet />}
              active={view === "timesheet"}
              onClick={() => setView("timesheet")}
              badge={pendingDays || undefined}
              badgeTitle="Bu hafta aktarılmamış gün"
            >
              Zaman çizelgesi
            </NavItem>
          </NavSection>
          <NavSection title="Raporlar">
            {REPORTS.map((r) => (
              <NavItem key={r.id} icon={r.icon} active={view === r.id} onClick={() => selectMode(r.id)}>
                {r.label}
              </NavItem>
            ))}
            <NavItem icon={<TrendingUp />} active={view === "trends"} onClick={() => setView("trends")}>
              Eğilimler
            </NavItem>
            <NavItem icon={<ReceiptText />} active={view === "client-report"} onClick={() => setView("client-report")}>
              Müşteri raporu
            </NavItem>
            <NavItem icon={<SearchIcon />} active={view === "search"} onClick={() => setView("search")}>
              Ara
            </NavItem>
          </NavSection>
          <NavSection title="Düzenle">
            <NavItem icon={<Building2 />} active={view === "clients"} onClick={() => setView("clients")}>
              Müşteriler
            </NavItem>
            <NavItem
              icon={<FolderKanban />}
              active={view === "projects"}
              onClick={() => setView("projects")}
              badge={suggestionCount.projects || undefined}
              badgeTitle="Bekleyen proje önerisi"
            >
              Projeler
            </NavItem>
            <NavItem
              icon={<Tags />}
              active={view === "categories"}
              onClick={() => setView("categories")}
              badge={suggestionCount.categories || undefined}
              badgeTitle="Bekleyen kategori önerisi"
            >
              Kategoriler
            </NavItem>
            <NavItem icon={<Settings2 />} active={view === "settings"} onClick={() => openSettings()}>
              Ayarlar
            </NavItem>
          </NavSection>
        </nav>
        <div className="space-y-2 p-2.5">
          <UpdateCard status={update} onStatus={setUpdate} />
          <LiveCard tracking={tracking} dailyHours={dailyHours} onToggle={togglePause} />
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col bg-background mica:bg-background/75">
        {view === "day" || view === "week" || view === "month" ? (
          <ReportView
            mode={view}
            start={view === "day" ? day : view === "week" ? week : month}
            title={title}
            onMode={selectMode}
            onPrev={() => step(-1)}
            onNext={() => step(1)}
            onToday={atCurrent ? null : goToday}
            onSelectDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
            onReview={() =>
              openReview(
                view === "day"
                  ? { start: day, days: 1 }
                  : view === "week"
                    ? { start: week, days: 7 }
                    : { start: month, days: daysInMonth(parseIsoDate(month)) },
              )
            }
          />
        ) : view === "timesheet" ? (
          // Zaman çizelgesi üst çubuğu kendisi çizer (dönem ve görünüm denetimleriyle).
          <Timesheet
            onOpenDay={(iso) => {
              setDay(iso);
              setView("day");
            }}
            onReviewDay={(iso) => openReview({ start: iso, days: 1 })}
            onOpenSettings={openSettings}
          />
        ) : (
          <>
            <Toolbar title={TITLES[view] ?? ""} />
            <div key={view} className="page-enter flex-1 overflow-y-auto">
              {view === "review" && (
                <Review
                  key={reviewRange ? `${reviewRange.start}/${reviewRange.days}` : "own"}
                  range={reviewRange}
                  onOpenTimesheet={() => setView("timesheet")}
                  onOpenProjects={() => setView("projects")}
                />
              )}
              {view === "trends" && (
                <Trends
                  onAddProject={() => setView("projects")}
                  onSearch={(query) => {
                    setSearch({ query, days: 30 });
                    setView("search");
                  }}
                />
              )}
              {view === "client-report" && <ClientReport />}
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
              {view === "clients" && <ClientsPage onOpenProjects={() => setView("projects")} />}
              {view === "projects" && <TagsPage key="project" kind="project" onSuggestions={onSuggestions} />}
              {view === "categories" && <TagsPage key="category" kind="category" onSuggestions={onSuggestions} />}
              {view === "settings" && <Settings status={status} onChange={refresh} section={settingsSection} />}
            </div>
          </>
        )}
      </main>
      <CommandPalette
        open={palette}
        onClose={() => setPalette(false)}
        commands={commands}
        onSearch={(query) => {
          setSearch({ query, days: 30 });
          setView("search");
        }}
      />
      <Toaster />
    </div>
  );
}

/** Rozet için kısa süre: "45dk", "3sa", "12sa". */
function shortDuration(secs: number) {
  return secs >= 3600 ? `${Math.floor(secs / 3600)}sa` : `${Math.floor(secs / 60)}dk`;
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
  badgeTitle,
  tone,
  children,
}: {
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  /** Sağda küçük rozet (örn. bekleyen öneri sayısı, atanmamış süre). */
  badge?: number | string;
  badgeTitle?: string;
  /** "brand": rozet kum renginde (dikkat isteyen iş). */
  tone?: "brand";
  children: ReactNode;
}) {
  const label = typeof children === "string" ? children : undefined;
  return (
    <button
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      // Rozet yalnızca bir sayı ya da süre; ekran okuyucu ne olduğunu da duysun.
      aria-label={label && badge ? `${label}, ${badgeTitle ?? "rozet"}: ${badge}` : undefined}
      title={label}
      className={cn(
        "group/nav relative flex h-7 w-full items-center gap-2 rounded-md px-2 text-[13px] transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/50 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:transition-colors",
        active
          ? "bg-sidebar-accent font-medium [&_svg]:text-primary"
          : "hover:bg-sidebar-accent/50 [&_svg]:text-muted-foreground hover:[&_svg]:text-foreground/80",
      )}
    >
      {active && <span className="absolute top-1.5 bottom-1.5 left-0 w-[3px] rounded-full bg-primary" aria-hidden />}
      {icon}
      {/* Dar kenar çubuğunda ad tek satırda kalır, sığmazsa kısalır (iki satıra bölünüp ortalanmaz). */}
      <span className="min-w-0 flex-1 truncate text-left">{children}</span>
      {!!badge && (
        <span
          title={badgeTitle}
          aria-label={badgeTitle ? `${badgeTitle}: ${badge}` : undefined}
          className={cn(
            "shrink-0 rounded-full px-1.5 text-[10px] leading-4 font-semibold tabular",
            tone === "brand" ? "bg-brand text-white shadow-sm shadow-brand-2/30" : "bg-primary/15 text-primary",
          )}
        >
          {badge}
        </span>
      )}
    </button>
  );
}

/** Kenar çubuğunun altında: şu an ne takip ediliyor, bugünkü toplam ve hedef, duraklat. */
function LiveCard({
  tracking,
  dailyHours,
  onToggle,
}: {
  tracking: TrackingStatus;
  dailyHours: number;
  onToggle: () => void;
}) {
  const [menu, setMenu] = useState(false);
  const project = tracking.paused || tracking.needsPermission ? null : (tracking.current?.project ?? null);
  const state = tracking.paused
    ? "Duraklatıldı"
    : tracking.needsPermission
      ? "İzin gerekli"
      : tracking.current
        ? tracking.current.appName
        : "Boşta";
  const live = !tracking.paused && !tracking.needsPermission && !!tracking.current;
  const goal = dailyHours * 3600;
  const ratio = goal > 0 ? Math.min(1, tracking.todaySeconds / goal) : 0;
  return (
    <div
      className={cn(
        "rounded-xl border border-sidebar-border bg-background/70 p-2.5 shadow-xs [--live-bg:var(--background)] dark:bg-white/5 dark:[--live-bg:#232325]",
        live && "live-border",
      )}
    >
      <div className="flex items-start gap-2">
        <span className="relative mt-[5px] flex size-2 shrink-0">
          {live && <span className="absolute inline-flex size-full animate-ping rounded-full bg-success opacity-60" />}
          <span
            className={cn(
              "relative inline-flex size-2 rounded-full",
              live ? "bg-success" : tracking.paused ? "bg-amber-500" : "bg-muted-foreground/50",
            )}
          />
        </span>
        <div className="min-w-0 flex-1">
          {project ? (
            <div className="flex min-w-0 items-center gap-1.5 text-xs font-medium" title={`Proje: ${project.name}`}>
              <i className="size-2 shrink-0 rounded-full" style={{ background: `var(--c${project.color})` }} />
              <span className="min-w-0 truncate">{project.name}</span>
              <span className="shrink-0 font-normal text-muted-foreground tabular">
                · {formatDuration(project.secondsToday)}
              </span>
            </div>
          ) : (
            <div className="truncate text-xs font-medium">{state}</div>
          )}
          {tracking.paused && tracking.pausedUntil && (
            <div className="text-[11px] text-muted-foreground tabular">
              Devam: {formatTime(new Date(tracking.pausedUntil))}
            </div>
          )}
          {live && (project || tracking.current?.title) && (
            <div className="truncate text-[11px] text-muted-foreground" title={tracking.current?.title || undefined}>
              {project
                ? [tracking.current?.appName, tracking.current?.title].filter(Boolean).join(" — ")
                : tracking.current?.title}
            </div>
          )}
        </div>
      </div>
      <div className="mt-2 flex items-end justify-between">
        <div>
          <div className="text-[11px] text-muted-foreground">Bugün</div>
          <div className="text-[17px] leading-tight font-semibold tracking-tight tabular">
            {formatDuration(tracking.todaySeconds)}
          </div>
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
                  className="flex h-7 w-full items-center rounded-md px-2 text-left text-xs outline-none hover:bg-accent focus-visible:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50"
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
      {goal > 0 && (
        <div
          className="mt-2 h-1 overflow-hidden rounded-full bg-muted"
          title={`Günlük hedef: ${formatDuration(goal)} · %${Math.round(ratio * 100)}`}
        >
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-700",
              ratio >= 1 ? "bg-success" : "bg-brand",
            )}
            style={{ width: `${ratio * 100}%` }}
          />
        </div>
      )}
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
