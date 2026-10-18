import { type ReactNode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Plus, Search, X } from "lucide-react";
import { api, type AppIcon, type AppStatus, type PrivacySettings, type ShortcutStatus, type UsageTotal } from "../api";
import { ErrorText, SettingBlock, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { cn } from "../lib/utils";
import { AiSettings } from "./AiSettings";
import { ConnectionsSettings } from "./ConnectionSettings";
import GoalsSettings from "./GoalsSettings";
import SyncSettings from "./SyncSettings";
import DataHealthSettings from "./DataHealthSettings";
import { AI_SECTION, CONNECTIONS_SECTION, TIMESHEET_SECTION, TimesheetSections } from "./TimesheetSettings";
import UpdateSettings from "./UpdateSettings";
import BackupSettings from "./BackupSettings";
import { friendlyError, toast } from "../lib/feedback";
import { saveStartView, savedStartView, type StartView } from "../lib/startView";

/** Soldaki içindekiler: üst başlıklar ve altlarındaki bölümler, sayfadaki sırasıyla. */
const GROUPS = [
  {
    title: "Temel",
    sections: [
      { id: "genel", label: "Genel" },
      { id: "gorunum", label: "Görünüm" },
    ],
  },
  {
    title: "Takip ve gizlilik",
    sections: [
      { id: "takip", label: "Takip" },
      { id: "gizlilik", label: "Gizlilik" },
    ],
  },
  {
    title: "İş akışı",
    sections: [
      { id: TIMESHEET_SECTION, label: "Zaman çizelgeleri" },
      { id: CONNECTIONS_SECTION, label: "Bağlantılar" },
      { id: AI_SECTION, label: "Yapay zekâ" },
    ],
  },
  { title: "Bildirimler", sections: [{ id: "hedefler", label: "Hedefler ve hatırlatıcılar" }] },
  {
    title: "Hesap ve veri",
    sections: [
      { id: "senkronizasyon", label: "Senkronizasyon" },
      { id: "veriler", label: "Veriler ve yedek" },
      { id: "hakkinda", label: "Hakkında ve destek" },
    ],
  },
];

/** Boşta kaydının en uzun süresi seçenekleri (dakika). */
const IDLE_MAX_OPTIONS = [60, 120, 180, 240, 360, 480];

/** Uygulama simgesi seçenekleri; görseller `public/` altında. */
const APP_ICONS: { id: AppIcon; label: string; src: string }[] = [
  { id: "kum", label: "Kum", src: "/icon.png" },
  { id: "kobalt", label: "Kobalt", src: "/icon-kobalt.png" },
];

function scrollTo(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
}

/** Sayfayı kaydıran ilk üst öğe (uygulamanın içerik alanı). */
function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement; p; p = p.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
  }
  return null;
}

const norm = (s: string) => s.toLocaleLowerCase("tr");

/**
 * Aramayı sayfaya uygular: eşleşmeyen satırları, satırı kalmayan bölümleri ve üst başlıkları
 * gizler. Bölüm adı eşleşirse bölümün tamamı görünür. Satırlar DOM'dan okunur; böylece alt
 * bileşenlerin (hedefler, senkronizasyon…) kendi satırları da ayrıca işaretlenmeden aranır.
 */
function applyFilter(root: HTMLElement, q: string) {
  let count = 0;
  const hidden: string[] = [];
  root.querySelectorAll<HTMLElement>("[data-settings-super]").forEach((sup) => {
    let any = false;
    sup.querySelectorAll<HTMLElement>("[data-settings-group]").forEach((group) => {
      const titleHit = !q || norm(group.dataset.settingsGroup ?? "").includes(q);
      let n = 0;
      const rows = group.querySelector("[data-settings-rows]")?.children ?? [];
      for (const row of Array.from(rows) as HTMLElement[]) {
        const hit = titleHit || norm(row.textContent ?? "").includes(q);
        row.hidden = !hit;
        if (hit) n++;
      }
      group.hidden = n === 0 && !!q;
      if (group.hidden) hidden.push(group.id);
      else any = true;
      if (q) count += n;
    });
    // Arama yokken hepsi görünür: bölümleri henüz yüklenmemiş başlık da gizli kalmasın.
    sup.hidden = !!q && !any;
  });
  return { count, hidden: hidden.join(" ") };
}

export default function Settings({
  status,
  onChange,
  section,
}: {
  status: AppStatus;
  onChange: () => void;
  /** Açılınca gidilecek bölüm (örn. zaman çizelgesindeki "Ayarlar" düğmesi). */
  section?: string | null;
}) {
  const [privacy, setPrivacy] = useState<PrivacySettings | null>(null);
  const [startView, setStartView] = useState<StartView>(savedStartView);
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [diag, setDiag] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState<string | null>(null);
  const [shortcut, setShortcut] = useState<ShortcutStatus | null>(null);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState({ count: 0, hidden: "" });
  const [active, setActive] = useState<string>("genel");
  const content = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const mac = status.platform === "macos";
  // Ekranı uyanık tutan video ve görüşmenin mikrofonu macOS ve Windows'ta okunur.
  const signals = mac || status.platform === "windows";

  useEffect(() => {
    // Gizlilik okunamazsa bölüm sessizce kaybolmasın.
    api.privacy().then(setPrivacy, (e) => setError(friendlyError(e)));
    api.knownApps().then(setApps, () => {});
    api.shortcutStatus().then(setShortcut, () => {});
  }, []);
  useEffect(() => {
    if (!section) return;
    // Bazı bölümler (çizelgeler) ayarlar okununca çizilir: bölüm görünene kadar bekle, üstündeki
    // bölümler de yüklenip yeri kaydırabileceği için kısa süre sonra bir kez daha git.
    let tries = 0;
    let settle: number | undefined;
    const timer = window.setInterval(() => {
      const el = document.getElementById(section);
      if (!el && ++tries < 30) return;
      window.clearInterval(timer);
      el?.scrollIntoView({ block: "start" });
      settle = window.setTimeout(() => el?.scrollIntoView({ block: "start" }), 300);
    }, 50);
    return () => {
      window.clearInterval(timer);
      window.clearTimeout(settle);
    };
  }, [section]);

  // İçindekilerde şu an okunan bölüm: üstü arama çubuğunun altına gelmiş son bölüm.
  const spy = useCallback(() => {
    const root = content.current;
    const sc = scrollParent(root);
    if (!root || !sc) return;
    const groups = Array.from(root.querySelectorAll<HTMLElement>("[data-settings-group]")).filter(
      (g) => g.getClientRects().length > 0,
    );
    if (!groups.length) return;
    const line = sc.getBoundingClientRect().top + 80;
    let on = groups[0];
    for (const g of groups) if (g.getBoundingClientRect().top <= line) on = g;
    if (sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 4) on = groups[groups.length - 1];
    setActive(on.id);
  }, []);
  useEffect(() => {
    const sc = scrollParent(content.current);
    sc?.addEventListener("scroll", spy, { passive: true });
    spy();
    return () => sc?.removeEventListener("scroll", spy);
  }, [spy]);

  // Arama: sonradan yüklenen satırlar (çizelgeler, senkronizasyon…) da süzülsün diye DOM izlenir.
  useLayoutEffect(() => {
    const root = content.current;
    if (!root) return;
    const q = norm(query.trim());
    const run = () => {
      const next = applyFilter(root, q);
      setFound((f) => (f.count === next.count && f.hidden === next.hidden ? f : next));
      spy();
    };
    run();
    if (!q) return;
    const observer = new MutationObserver(run);
    observer.observe(root, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [query, spy]);

  // ⌘F / Ctrl+F aramaya gider.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        search.current?.focus();
        search.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  async function save(next: PrivacySettings) {
    try {
      setError(null);
      // Arka uç geçersiz adres kalıplarını ayıklar: ekranda kaydedilen hal görünsün.
      setPrivacy(await api.savePrivacy(next));
    } catch (e) {
      setError(friendlyError(e));
    }
  }

  async function toggleAutostart() {
    await api.setAutostart(!status.autostart);
    onChange();
  }

  async function runDiagnostics() {
    const lines: string[] = [];
    setDiag(lines);
    for (let i = 0; i < 5; i++) {
      lines.push(await api.diagnose());
      setDiag([...lines]);
      await new Promise((r) => setTimeout(r, 1000));
    }
  }

  const searching = query.trim() !== "";
  const hidden = new Set(found.hidden.split(" "));

  return (
    <div className="@container">
      <h1 className="sr-only">Ayarlar</h1>
      <div className="mx-auto flex w-full max-w-[1000px] gap-8 px-6 pb-10">
        <nav
          aria-label="Ayarlar bölümleri"
          className="sticky top-0 hidden max-h-[calc(100vh-3rem)] w-44 shrink-0 self-start overflow-y-auto pt-5 pb-4 @[880px]:block"
        >
          {GROUPS.map((g) => (
            <div key={g.title} className="mb-3">
              <div className="px-2 pb-1 text-[10.5px] font-semibold tracking-wider text-muted-foreground uppercase">
                {g.title}
              </div>
              {g.sections.map((s) => {
                const off = searching && hidden.has(s.id);
                return (
                  <button
                    key={s.id}
                    disabled={off}
                    onClick={() => scrollTo(s.id)}
                    aria-current={active === s.id ? "location" : undefined}
                    className={cn(
                      "block w-full truncate rounded-md px-2 py-1 text-left text-[13px] text-muted-foreground hover:text-foreground disabled:pointer-events-none disabled:opacity-35",
                      active === s.id && "bg-primary/10 font-medium text-primary hover:text-primary",
                    )}
                  >
                    {s.label}
                  </button>
                );
              })}
            </div>
          ))}
        </nav>

        <div ref={content} className="mx-auto flex max-w-3xl min-w-0 flex-1 flex-col gap-10">
          <div className="sticky top-0 z-10 -mx-1 bg-background px-1 pt-3 pb-2">
            <label className="flex h-9 items-center gap-2 rounded-lg border bg-card px-3 shadow-xs focus-within:ring-2 focus-within:ring-ring/50">
              <Search className="size-4 shrink-0 text-muted-foreground" />
              <input
                ref={search}
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                placeholder="Ayarlarda ara: boşta, yedek, Outlook…"
                aria-label="Ayarlarda ara"
                autoComplete="off"
                spellCheck={false}
                className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
              />
              {searching && (
                <>
                  <span className="text-xs whitespace-nowrap text-muted-foreground">{found.count} ayar</span>
                  <button
                    onClick={() => setQuery("")}
                    aria-label="Aramayı temizle"
                    className="grid size-5 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
                  >
                    <X className="size-3.5" />
                  </button>
                </>
              )}
            </label>
            <ErrorText>{error}</ErrorText>
          </div>
          {searching && found.count === 0 && (
            <p className="py-10 text-center text-sm text-muted-foreground">“{query.trim()}” için ayar bulunamadı.</p>
          )}

          <SettingsSuper title="Temel">
            <SettingsGroup id="genel" title="Genel">
              <ToggleRow
                label="Bilgisayar açılınca başlat"
                hint="Kum menü çubuğunda sessizce başlar."
                checked={status.autostart}
                onChange={toggleAutostart}
              />
              {shortcut && (
                <ToggleRow
                  label={`Son süreyi ata kısayolu (${shortcut.label})`}
                  hint={
                    shortcut.error
                      ? `Kısayol kaydedilemedi: ${shortcut.error}. Başka bir uygulama kullanıyor olabilir.`
                      : "Hangi uygulamada olursan ol son 15–120 dakikayı bir projeye yazdırır."
                  }
                  checked={shortcut.enabled}
                  onChange={(on) => api.setShortcutEnabled(on).then(setShortcut, (e) => setError(friendlyError(e)))}
                />
              )}
              <SettingRow label="Açılış sayfası" hint="Kum açılınca gösterilen sayfa.">
                <Select
                  value={startView}
                  onValueChange={(v) => {
                    setStartView(v as StartView);
                    saveStartView(v as StartView);
                  }}
                >
                  <SelectTrigger size="sm" className="w-32" aria-label="Açılış sayfası">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    <SelectItem value="day">Gün</SelectItem>
                    <SelectItem value="home">Bugün</SelectItem>
                  </SelectContent>
                </Select>
              </SettingRow>
            </SettingsGroup>

            <SettingsGroup id="gorunum" title="Görünüm">
              <SettingRow label="Tema" hint="Sistem seçiliyken bilgisayarın açık/koyu ayarını izler.">
                <Select
                  value={status.theme}
                  onValueChange={(v) =>
                    api.setTheme(v as AppStatus["theme"]).then(onChange, (e) => setError(friendlyError(e)))
                  }
                >
                  <SelectTrigger size="sm" className="w-32" aria-label="Tema">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    <SelectItem value="system">Sistem</SelectItem>
                    <SelectItem value="light">Açık</SelectItem>
                    <SelectItem value="dark">Koyu</SelectItem>
                  </SelectContent>
                </Select>
              </SettingRow>
              <SettingRow
                label="Uygulama simgesi"
                hint={
                  mac
                    ? "Dock’ta ve uygulama değiştiricide görünür; Finder’da Kum simgesi kalır."
                    : "Görev çubuğunda görünür; kısayollarda Kum simgesi kalır."
                }
              >
                <div role="group" aria-label="Uygulama simgesi" className="flex gap-1">
                  {APP_ICONS.map((icon) => {
                    const selected = status.appIcon === icon.id;
                    return (
                      <button
                        key={icon.id}
                        type="button"
                        aria-pressed={selected}
                        onClick={() => api.setAppIcon(icon.id).then(onChange, (e) => setError(friendlyError(e)))}
                        className={cn(
                          "flex w-14 flex-col items-center gap-0.5 rounded-lg py-1 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground",
                          selected && "bg-accent font-medium text-foreground ring-1 ring-primary/40",
                        )}
                      >
                        <img src={icon.src} alt="" className="size-10" />
                        {icon.label}
                      </button>
                    );
                  })}
                </div>
              </SettingRow>
            </SettingsGroup>
          </SettingsSuper>

          <SettingsSuper title="Takip ve gizlilik">
            {(privacy || mac) && (
              <SettingsGroup id="takip" title="Takip" description="Kum’un neyi çalışma sayacağı.">
                {privacy && (
                  <>
                    <ToggleRow
                      label="Boşta geçen süreyi takvimde göster"
                      hint="3 dakika girdi yoksa ya da uykudayken geçen süre “Boşta” olarak görünür. Çalışmaya sayılmaz; tıklayıp bir projeye atarsan sayılır."
                      checked={privacy.record_idle}
                      onChange={(v) => save({ ...privacy, record_idle: v })}
                    />
                    {privacy.record_idle && (
                      <SettingRow sub label="En uzun boşluk" hint="Bundan uzun boşluklar (gece gibi) kaydedilmez.">
                        <Select
                          value={String(privacy.idle_max_minutes)}
                          onValueChange={(v) => save({ ...privacy, idle_max_minutes: Number(v) })}
                        >
                          <SelectTrigger size="sm" className="w-32" aria-label="En uzun boşluk">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent align="end">
                            {IDLE_MAX_OPTIONS.map((m) => (
                              <SelectItem key={m} value={String(m)}>
                                {m / 60} saat
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </SettingRow>
                    )}
                    {signals && (
                      <ToggleRow
                        label="İzlerken boşta sayma"
                        hint={`Video oynarken ya da görüntülü görüşmedeyken klavyeye dokunmasan da süre çalışma sayılır (en çok 3 saat). Ekranı açık tutan araçlar (${mac ? "caffeinate, Amphetamine" : "PowerToys Awake"}) tek başına sayılmaz.`}
                        checked={privacy.count_watching}
                        onChange={(v) => save({ ...privacy, count_watching: v })}
                      />
                    )}
                    {signals && (
                      <ToggleRow
                        label="Görüşmeleri algıla"
                        hint="Teams, Zoom ya da Meet mikrofonu kullanırken bunun ne zaman olduğu kaydedilir, ses kaydedilmez. Toplantı zaman çizelgesine gerçek süresiyle girer; katılmadığın toplantı girmez."
                        checked={privacy.detect_calls}
                        onChange={(v) => save({ ...privacy, detect_calls: v })}
                      />
                    )}
                  </>
                )}
                {mac && (
                  <SettingRow label="Erişilebilirlik izni" hint="Pencere başlıklarını okumak için gerekir.">
                    {status.accessibility ? (
                      <Badge variant="success">Verildi</Badge>
                    ) : (
                      <Button variant="outline" size="sm" onClick={() => api.openAccessibilitySettings()}>
                        Ayarları Aç
                      </Button>
                    )}
                  </SettingRow>
                )}
              </SettingsGroup>
            )}

            {privacy && (
              <SettingsGroup
                id="gizlilik"
                title="Gizlilik"
                description="Değişiklikler yeni kayıtlara uygulanır; geçmiş kayıtlar değişmez."
              >
                <ToggleRow
                  label="Gizli pencerelerin başlığını kaydetme"
                  hint="Tarayıcıların gizli/InPrivate pencerelerinde süre kaydedilir, başlık “Gizli” olarak saklanır."
                  checked={privacy.hide_private_windows}
                  onChange={(v) => save({ ...privacy, hide_private_windows: v })}
                />
                <AppPicker
                  title="Hiç kaydedilmeyen uygulamalar"
                  hint="Bu uygulamalarda geçen süre hiç kaydedilmez (örn. şifre yöneticileri)."
                  selected={privacy.excluded_apps}
                  apps={apps}
                  onChange={(excluded_apps) => save({ ...privacy, excluded_apps })}
                />
                <UrlEditor
                  urls={privacy.excluded_urls ?? []}
                  onChange={(excluded_urls) => save({ ...privacy, excluded_urls })}
                />
                <AppPicker
                  title="Başlığı kaydedilmeyen uygulamalar"
                  hint="Süre kaydedilir ama pencere başlığı “Gizli” olarak saklanır (örn. e-posta)."
                  selected={privacy.hidden_title_apps}
                  apps={apps}
                  onChange={(hidden_title_apps) => save({ ...privacy, hidden_title_apps })}
                />
                <SuffixEditor
                  suffixes={privacy.title_suffixes ?? []}
                  onChange={(title_suffixes) => save({ ...privacy, title_suffixes })}
                />
              </SettingsGroup>
            )}
          </SettingsSuper>

          <SettingsSuper title="İş akışı">
            <TimesheetSections />
            <ConnectionsSettings />
            <AiSettings />
          </SettingsSuper>

          <SettingsSuper title="Bildirimler">
            <GoalsSettings />
          </SettingsSuper>

          <SettingsSuper title="Hesap ve veri">
            <SyncSettings />

            <SettingsGroup id="veriler" title="Veriler ve yedek">
              <SettingRow
                label="CSV olarak dışa aktar"
                hint={
                  exported ??
                  "Tüm kayıtlar (başlangıç, bitiş, uygulama, başlık, kategori, proje) İndirilenler klasörüne yazılır."
                }
              >
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    api.exportCsv().then(
                      (p) => setExported(`Kaydedildi: ${p}`),
                      (e) => setError(friendlyError(e)),
                    )
                  }
                >
                  Dışa aktar
                </Button>
              </SettingRow>
              <BackupSettings />
            </SettingsGroup>

            <SettingsGroup id="hakkinda" title="Hakkında ve destek">
              <UpdateSettings />
              <DataHealthSettings />
              <SettingRow
                label="Tanılama"
                hint="Pencere başlıkları görünmüyorsa çalıştır, 5 saniye boyunca farklı pencerelere geç ve çıkan metni paylaş."
              >
                <Button variant="outline" size="sm" onClick={runDiagnostics}>
                  Çalıştır
                </Button>
              </SettingRow>
              {diag && (
                <pre className="max-h-56 overflow-auto bg-muted/50 px-4 py-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap">
                  {diag.join("\n")}
                </pre>
              )}
              <SettingRow
                label="Hata günlüğü"
                hint="Bir sorun bildirirken sürümü, izinleri, takip ve eşitleme durumunu ve son hataları panoya kopyalar. Anahtar ve jeton içermez."
              >
                <div className="flex gap-2">
                  <Button variant="ghost" size="sm" onClick={() => api.revealLog()}>
                    Dosyayı göster
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      api
                        .diagnostics()
                        .then((text) => navigator.clipboard.writeText(text))
                        .then(
                          () => toast("Tanılama bilgisi kopyalandı", { tone: "success" }),
                          (e) => setError(friendlyError(e)),
                        )
                    }
                  >
                    Kopyala
                  </Button>
                </div>
              </SettingRow>
            </SettingsGroup>
          </SettingsSuper>
        </div>
      </div>
    </div>
  );
}

/** Ayarlardaki üst başlık (Temel, İş akışı…) ve altındaki bölümler. */
function SettingsSuper({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div data-settings-super className="flex flex-col gap-7">
      <h2 className="-mb-2 px-1 text-[19px] font-semibold tracking-tight">{title}</h2>
      {children}
    </div>
  );
}

function Chips({
  items,
  onRemove,
}: {
  items: { key: string; label: string; title?: string }[];
  onRemove: (key: string) => void;
}) {
  if (items.length === 0) return <p className="text-xs text-muted-foreground">Yok</p>;
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map((i) => (
        <Badge key={i.key} variant="secondary" className="gap-1 py-0.5 pr-1 pl-2 text-xs font-normal" title={i.title}>
          {i.label}
          <button
            className="grid size-4 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
            onClick={() => onRemove(i.key)}
            aria-label="Kaldır"
          >
            <X className="size-3" />
          </button>
        </Badge>
      ))}
    </div>
  );
}

function AppPicker({
  title,
  hint,
  selected,
  apps,
  onChange,
}: {
  title: string;
  hint: string;
  selected: string[];
  apps: UsageTotal[];
  onChange: (ids: string[]) => void;
}) {
  const names = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);
  const available = apps.filter((a) => !selected.includes(a.key));
  return (
    <SettingBlock label={title} hint={hint}>
      <Chips
        items={selected.map((id) => ({ key: id, label: names.get(id) ?? id, title: id }))}
        onRemove={(id) => onChange(selected.filter((s) => s !== id))}
      />
      <Select value="" onValueChange={(v) => v && onChange([...selected, v])} disabled={available.length === 0}>
        <SelectTrigger
          size="sm"
          className="w-56 [&>[data-slot=select-value]]:flex-1"
          aria-label={`${title}: uygulama ekle`}
        >
          <Plus className="size-3.5" />
          <SelectValue placeholder="Uygulama ekle…" />
        </SelectTrigger>
        <SelectContent>
          {available.map((a) => (
            <SelectItem key={a.key} value={a.key}>
              {a.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </SettingBlock>
  );
}

function UrlEditor({ urls, onChange }: { urls: string[]; onChange: (u: string[]) => void }) {
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  return (
    <SettingBlock
      label="Takip edilmeyen adresler"
      hint="Tarayıcıda bu sitelerde geçen süre hiç kaydedilmez; alt alan adları da kapsanır. Varsayılan olarak yetişkin sitelerini içerir."
    >
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        {urls.length} adres
        <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => setOpen((o) => !o)}>
          {open ? "Gizle" : "Göster"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-2 text-xs"
          onClick={() => api.defaultExcludedUrls().then((d) => onChange([...new Set([...urls, ...d])]))}
        >
          Varsayılanları ekle
        </Button>
      </div>
      {open && (
        <div className="max-h-40 overflow-auto">
          <Chips
            items={urls.map((u) => ({ key: u, label: u }))}
            onRemove={(u) => onChange(urls.filter((x) => x !== u))}
          />
        </div>
      )}
      <form
        className="flex max-w-sm gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) onChange([...urls, text.trim()]);
          setText("");
        }}
      >
        <Input
          className="h-7 text-xs"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="örn. site.com"
        />
        <Button type="submit" variant="outline" size="sm" disabled={!text.trim()}>
          Ekle
        </Button>
      </form>
    </SettingBlock>
  );
}

function SuffixEditor({ suffixes, onChange }: { suffixes: string[]; onChange: (s: string[]) => void }) {
  const [text, setText] = useState("");
  return (
    <SettingBlock
      label="Başlıklardan kaldırılacak ekler"
      hint="Bazı uygulamalar başlığın sonuna sabit bir şey ekler (örn. Firefox profil adı “— Kaan”). Buraya yazdığın ek başlıklardan silinir."
    >
      <Chips
        items={suffixes.map((s) => ({ key: s, label: s }))}
        onRemove={(s) => onChange(suffixes.filter((x) => x !== s))}
      />
      <form
        className="flex max-w-sm gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) onChange([...suffixes, text.trim()]);
          setText("");
        }}
      >
        <Input className="h-7 text-xs" value={text} onChange={(e) => setText(e.target.value)} placeholder="örn. Kaan" />
        <Button type="submit" variant="outline" size="sm" disabled={!text.trim()}>
          Ekle
        </Button>
      </form>
    </SettingBlock>
  );
}
