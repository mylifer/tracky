import type { ReactNode } from "react";
import {
  Building2,
  CalendarCheck,
  CalendarDays,
  CalendarRange,
  Calendar as CalendarIcon,
  ClockArrowDown,
  Download,
  FileSpreadsheet,
  FolderKanban,
  HardDriveDownload,
  History,
  House,
  Inbox,
  Monitor,
  Moon,
  Pause,
  Play,
  ReceiptText,
  RefreshCw,
  Search as SearchIcon,
  Settings2,
  Sun,
  Tags,
  TrendingUp,
} from "lucide-react";
import { api, type Tag } from "./api";
import type { Command } from "./components/CommandPalette";
import { friendlyError, toast } from "./lib/feedback";
import type { ThemePref } from "./lib/theme";
import type { Mode, View } from "./lib/views";

/** Mac'te ⌘, Windows'ta Ctrl ile kısayollar; ipuçlarında da böyle yazılır. */
const isMacUA = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
const MOD = isMacUA ? "⌘" : "Ctrl+";

/** Komut paletinin komutları: sayfalar, eylemler ve (aramaya götüren) projeler. */
export function appCommands({
  selectMode,
  setView,
  goToday,
  openAssignRecent,
  openHistory,
  paused,
  togglePause,
  changeTheme,
  projects,
  searchProject,
}: {
  selectMode: (m: Mode) => void;
  setView: (v: View) => void;
  goToday: () => void;
  openAssignRecent: () => void;
  openHistory: () => void;
  paused: boolean;
  togglePause: () => void;
  changeTheme: (t: ThemePref) => void;
  projects: Tag[];
  searchProject: (name: string) => void;
}): Command[] {
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
    go("home", "Bugün", <House />, undefined, "anasayfa özet sırada yapılacaklar"),
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
    act(
      "assign-recent",
      "Son süreyi projeye ata…",
      <ClockArrowDown />,
      openAssignRecent,
      "son dakika geriye dönük ata proje",
    ),
    act("history", "Değişiklik geçmişi", <History />, openHistory, "geri al undo son değişiklikler"),
    ...(paused
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
        searchProject(p.name);
      },
    })),
  ];
}
