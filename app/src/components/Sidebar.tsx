import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Building2,
  CalendarDays,
  CalendarRange,
  Calendar as CalendarIcon,
  Command as CommandIcon,
  FileSpreadsheet,
  FolderKanban,
  House,
  Inbox,
  Pause,
  Play,
  ReceiptText,
  Search as SearchIcon,
  Settings2,
  Tags,
  TrendingUp,
} from "lucide-react";
import { api, formatDuration, type Suggestions, type TrackingStatus, type UpdateStatus } from "../api";
import { isoDate, formatTime, startOfWeek, today } from "../lib/dates";
import { useChanged } from "../lib/feedback";
import { cn } from "../lib/utils";
import type { Mode, View } from "../lib/views";
import { Button } from "./ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";
import { UpdateCard } from "./UpdateCard";

const REPORTS: { id: Mode; label: string; icon: ReactNode }[] = [
  { id: "day", label: "Gün", icon: <CalendarDays /> },
  { id: "week", label: "Hafta", icon: <CalendarRange /> },
  { id: "month", label: "Ay", icon: <CalendarIcon /> },
];

export type SidebarBadges = {
  suggestions: { projects: number; categories: number };
  /** Bu haftanın atanmamış süresi (saniye). */
  unassigned: number;
  /** Bu hafta aktarılmamış gün sayısı. */
  pendingDays: number;
  /** Sayfa öneri listesini yeniden yükleyince rozeti de günceller. */
  onSuggestions: (s: Suggestions) => void;
};

/**
 * Kenar çubuğu rozetleri: bekleyen öneriler, bu haftanın atanmamış süresi ve aktarılmamış
 * günleri. Görünüm değişince, bir düzenleme olunca ve saatte bir yenilenir.
 */
export function useSidebarBadges(view: View): SidebarBadges {
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
  return { suggestions: suggestionCount, unassigned, pendingDays, onSuggestions };
}

/** Ana pencerenin kenar çubuğu: sayfalar, rozetler, güncelleme ve canlı kart. */
export function Sidebar({
  isMac,
  view,
  badges,
  onView,
  onMode,
  onSettings,
  onPalette,
  update,
  onUpdate,
  tracking,
  dailyHours,
  onTogglePause,
}: {
  isMac: boolean;
  view: View;
  badges: SidebarBadges;
  onView: (v: View) => void;
  onMode: (m: Mode) => void;
  onSettings: () => void;
  onPalette: () => void;
  update: UpdateStatus | null;
  onUpdate: (s: UpdateStatus) => void;
  tracking: TrackingStatus;
  dailyHours: number;
  onTogglePause: () => void;
}) {
  return (
    <aside className="flex w-[216px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground material:bg-transparent">
      {/* macOS'ta pencere düğmeleri bu şeridin üstünde durur; Windows'ta yerel başlık
          çubuğu ad ve simgeyi zaten gösterir, burada tekrarlanmaz. */}
      <div data-tauri-drag-region className={cn("shrink-0", isMac ? "h-[52px]" : "h-3.5")} />
      <button
        onClick={onPalette}
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
          <NavItem icon={<House />} active={view === "home"} onClick={() => onView("home")}>
            Bugün
          </NavItem>
          <NavItem
            icon={<Inbox />}
            active={view === "review"}
            onClick={() => onView("review")}
            badge={badges.unassigned >= 60 ? shortDuration(badges.unassigned) : undefined}
            badgeTitle="Bu hafta projeye atanmamış süre"
            tone="brand"
          >
            Gözden geçir
          </NavItem>
          <NavItem
            icon={<FileSpreadsheet />}
            active={view === "timesheet"}
            onClick={() => onView("timesheet")}
            badge={badges.pendingDays || undefined}
            badgeTitle="Bu hafta aktarılmamış gün"
          >
            Zaman çizelgesi
          </NavItem>
        </NavSection>
        <NavSection title="Raporlar">
          {REPORTS.map((r) => (
            <NavItem key={r.id} icon={r.icon} active={view === r.id} onClick={() => onMode(r.id)}>
              {r.label}
            </NavItem>
          ))}
          <NavItem icon={<TrendingUp />} active={view === "trends"} onClick={() => onView("trends")}>
            Eğilimler
          </NavItem>
          <NavItem icon={<ReceiptText />} active={view === "client-report"} onClick={() => onView("client-report")}>
            Müşteri raporu
          </NavItem>
          <NavItem icon={<SearchIcon />} active={view === "search"} onClick={() => onView("search")}>
            Ara
          </NavItem>
        </NavSection>
        <NavSection title="Düzenle">
          <NavItem icon={<Building2 />} active={view === "clients"} onClick={() => onView("clients")}>
            Müşteriler
          </NavItem>
          <NavItem
            icon={<FolderKanban />}
            active={view === "projects"}
            onClick={() => onView("projects")}
            badge={badges.suggestions.projects || undefined}
            badgeTitle="Bekleyen proje önerisi"
          >
            Projeler
          </NavItem>
          <NavItem
            icon={<Tags />}
            active={view === "categories"}
            onClick={() => onView("categories")}
            badge={badges.suggestions.categories || undefined}
            badgeTitle="Bekleyen kategori önerisi"
          >
            Kategoriler
          </NavItem>
          <NavItem icon={<Settings2 />} active={view === "settings"} onClick={onSettings}>
            Ayarlar
          </NavItem>
        </NavSection>
      </nav>
      <div className="space-y-2 p-2.5">
        <UpdateCard status={update} onStatus={onUpdate} />
        <LiveCard tracking={tracking} dailyHours={dailyHours} onToggle={onTogglePause} />
      </div>
    </aside>
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
