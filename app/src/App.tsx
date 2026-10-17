import { useCallback, useEffect, useMemo, useState } from "react";
import { RotateCw } from "lucide-react";
import { api, type AppStatus, type Tag, type TrackingStatus } from "./api";
import { appCommands } from "./appCommands";
import ReportView from "./components/ReportView";
import { Sidebar, useSidebarBadges } from "./components/Sidebar";
import Toolbar from "./components/Toolbar";
import { Toaster } from "./components/Toaster";
import { CommandPalette, type Command } from "./components/CommandPalette";
import { Button } from "./components/ui/button";
import {
  addDays,
  addMonths,
  daysInMonth,
  formatMonth,
  formatWeek,
  isoDate,
  parseIsoDate,
  startOfMonth,
  startOfWeek,
  today,
} from "./lib/dates";
import { friendlyError } from "./lib/feedback";
import { activeProjects } from "./lib/tags";
import { applyPlatform, useTheme, type ThemePref } from "./lib/theme";
import { useUpdate } from "./lib/useUpdate";
import type { Mode, View } from "./lib/views";
import Onboarding from "./Onboarding";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { HistoryDialog } from "./components/HistoryDialog";
import { AssignRecentDialog } from "./components/AssignRecentDialog";
import TagsPage from "./pages/TagsPage";
import ClientsPage from "./pages/ClientsPage";
import ProjectsPage from "./pages/ProjectsPage";
import Search, { type SearchState } from "./pages/Search";
import Trends from "./pages/Trends";
import ClientReport from "./pages/ClientReport";
import Timesheet from "./pages/Timesheet";
import Review, { type ReviewRange } from "./pages/Review";
import Settings from "./pages/Settings";
import Today from "./pages/Today";
import { savedStartView } from "./lib/startView";
import { useTauriEvent } from "./lib/useTauriEvent";

const TITLES: Partial<Record<View, string>> = {
  home: "Bugün",
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
  const [view, setView] = useState<View>(savedStartView);
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
  // Projeler sayfasında açık proje (Müşteriler'den bir projeye gidince de); sayfadan çıkınca kapanır.
  const [openProject, setOpenProject] = useState<string | null>(null);
  useEffect(() => {
    if (view !== "projects") setOpenProject(null);
  }, [view]);
  // Kenar çubuğu rozetleri (bekleyen öneriler, atanmamış süre, aktarılmamış günler).
  const badges = useSidebarBadges(view);
  const onSuggestions = badges.onSuggestions;
  const [day, setDay] = useState(isoDate(today()));
  const [week, setWeek] = useState(isoDate(startOfWeek(today())));
  const [month, setMonth] = useState(isoDate(startOfMonth(today())));
  const [tracking, setTracking] = useState<TrackingStatus>(status.tracking);
  const [update, setUpdate] = useUpdate();
  const [palette, setPalette] = useState(false);
  const [history, setHistory] = useState(false);
  /** "Son süreyi ata" açık mı; kısayoldan ya da menü çubuğundan açıldıysa `outside`. */
  const [assignRecent, setAssignRecent] = useState<{ outside: boolean } | null>(null);
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
      case "history":
        return setHistory(true);
      case "assign-recent":
        return setAssignRecent({ outside: true });
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
      case "home":
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
      if (e.defaultPrevented) return;
      const mod = e.metaKey || e.ctrlKey;
      // ⌥⌘Z / Ctrl+Alt+Z: değişiklik geçmişi (Mac'te Düzen menüsü de aynı isteği gönderir).
      if (mod && e.altKey && e.code === "KeyZ") {
        e.preventDefault();
        setHistory(true);
        return;
      }
      if (e.altKey) return;
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

  const commands = useMemo<Command[]>(
    () =>
      appCommands({
        selectMode,
        setView,
        goToday,
        openAssignRecent: () => setAssignRecent({ outside: false }),
        openHistory: () => setHistory(true),
        paused: tracking.paused,
        togglePause,
        changeTheme,
        projects,
        searchProject: (name) => {
          setSearch({ query: name, days: 30 });
          setView("search");
        },
      }),
    // Palet açıkken güncel durum yeterli; her çizimde yeniden kurmak ucuz.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tracking.paused, projects, day, week, month, view],
  );

  const title =
    view === "day"
      ? capitalize(longDate.format(dayDate))
      : view === "week"
        ? formatWeek(parseIsoDate(week))
        : capitalize(formatMonth(parseIsoDate(month)));

  return (
    <div className="flex h-full">
      <Sidebar
        isMac={isMac}
        view={view}
        badges={badges}
        onView={setView}
        onMode={selectMode}
        onSettings={() => openSettings()}
        onPalette={() => setPalette(true)}
        update={update}
        onUpdate={setUpdate}
        tracking={tracking}
        dailyHours={dailyHours}
        onTogglePause={togglePause}
      />

      <main className="flex min-w-0 flex-1 flex-col bg-background mica:bg-background/75">
        <ErrorBoundary resetKey={view}>
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
          ) : view === "home" ? (
            // Bugün üst çubuğu kendisi çizer.
            <Today
              tracking={tracking}
              onTogglePause={togglePause}
              onOpenDay={(iso) => {
                setDay(iso);
                setView("day");
              }}
              onReview={openReview}
              onNavigate={setView}
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
                {view === "clients" && (
                  <ClientsPage
                    onOpenProjects={() => setView("projects")}
                    onOpenProject={(id) => {
                      setView("projects");
                      setOpenProject(id);
                    }}
                  />
                )}
                {view === "projects" && (
                  <ProjectsPage open={openProject} onOpen={setOpenProject} onSuggestions={onSuggestions} />
                )}
                {view === "categories" && <TagsPage key="category" kind="category" onSuggestions={onSuggestions} />}
                {view === "settings" && <Settings status={status} onChange={refresh} section={settingsSection} />}
              </div>
            </>
          )}
        </ErrorBoundary>
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
      <HistoryDialog open={history} onClose={() => setHistory(false)} />
      <AssignRecentDialog
        open={assignRecent !== null}
        fromOutside={assignRecent?.outside ?? false}
        onClose={() => setAssignRecent(null)}
      />
      <Toaster />
    </div>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
