import { useEffect, useMemo, useState } from "react";
import { Plus, X } from "lucide-react";
import { api, type AppStatus, type PrivacySettings, type UsageTotal } from "../api";
import { ErrorText, Page, SettingBlock, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { cn } from "../lib/utils";
import GoalsSettings from "./GoalsSettings";
import SyncSettings from "./SyncSettings";
import { AI_SECTION, CONNECTIONS_SECTION, TIMESHEET_SECTION, TimesheetSections } from "./TimesheetSettings";
import UpdateSettings from "./UpdateSettings";
import BackupSettings from "./BackupSettings";
import { friendlyError } from "../lib/feedback";

/** Sayfanın başındaki içindekiler: bölümler sayfadaki sırasıyla. */
const SECTIONS = [
  { id: "genel", label: "Genel" },
  { id: CONNECTIONS_SECTION, label: "Bağlantılar" },
  { id: TIMESHEET_SECTION, label: "Zaman çizelgeleri" },
  { id: AI_SECTION, label: "Yapay zekâ" },
  { id: "hedefler", label: "Hedefler" },
  { id: "gizlilik", label: "Gizlilik" },
  { id: "senkronizasyon", label: "Senkronizasyon" },
  { id: "guncellemeler", label: "Güncellemeler" },
  { id: "veriler", label: "Veriler" },
];

/** Boşta kaydının en uzun süresi seçenekleri (dakika). */
const IDLE_MAX_OPTIONS = [60, 120, 180, 240, 360, 480];

function scrollTo(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
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
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [diag, setDiag] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState<string | null>(null);

  useEffect(() => {
    // Gizlilik okunamazsa bölüm sessizce kaybolmasın.
    api.privacy().then(setPrivacy, (e) => setError(friendlyError(e)));
    api.knownApps().then(setApps, () => {});
  }, []);
  useEffect(() => {
    // Bölümler yüklenince yerleşsin diye bir kare sonra.
    if (section) requestAnimationFrame(() => scrollTo(section));
  }, [section]);

  async function save(next: PrivacySettings) {
    try {
      setError(null);
      await api.savePrivacy(next);
      setPrivacy(next);
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

  return (
    <Page title="Ayarlar">
      <nav aria-label="Ayarlar bölümleri" className="flex flex-wrap gap-1.5">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            onClick={() => scrollTo(s.id)}
            className={cn(
              "rounded-full border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground",
              section === s.id && "border-primary/40 text-foreground",
            )}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <ErrorText>{error}</ErrorText>

      <SettingsGroup id="genel" title="Genel">
        <ToggleRow
          label="Bilgisayar açılınca başlat"
          hint="Kum menü çubuğunda sessizce başlar."
          checked={status.autostart}
          onChange={toggleAutostart}
        />
        <SettingRow label="Görünüm" hint="Sistem seçiliyken bilgisayarın açık/koyu ayarını izler.">
          <Select
            value={status.theme}
            onValueChange={(v) =>
              api.setTheme(v as AppStatus["theme"]).then(onChange, (e) => setError(friendlyError(e)))
            }
          >
            <SelectTrigger size="sm" className="w-32" aria-label="Görünüm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="end">
              <SelectItem value="system">Sistem</SelectItem>
              <SelectItem value="light">Açık</SelectItem>
              <SelectItem value="dark">Koyu</SelectItem>
            </SelectContent>
          </Select>
        </SettingRow>
        {privacy && (
          <>
            <ToggleRow
              label="Boşta geçen süreyi takvimde göster"
              hint="Bilgisayardan uzaklaşınca (3 dakika girdi yoksa ya da uykudayken) geçen süre takvimde “Boşta” olarak görünür. Çalışma süresine sayılmaz; tıklayıp bir projeye atarsan sayılır."
              checked={privacy.record_idle}
              onChange={(v) => save({ ...privacy, record_idle: v })}
            />
            {privacy.record_idle && (
              <SettingRow label="En uzun boşluk" hint="Bundan uzun boşluklar (gece gibi) kaydedilmez.">
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
          </>
        )}
        {status.platform === "macos" && (
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

      <TimesheetSections />

      <GoalsSettings />

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

      <SyncSettings />

      <UpdateSettings />

      <SettingsGroup id="veriler" title="Veriler">
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

      <SettingsGroup id="sorun-giderme" title="Sorun giderme">
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
      </SettingsGroup>
    </Page>
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
        <SelectTrigger size="sm" className="w-56 [&>[data-slot=select-value]]:flex-1">
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
