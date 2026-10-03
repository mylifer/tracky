import { useEffect, useMemo, useState } from "react";
import { Plus, X } from "lucide-react";
import { api, type AppStatus, type PrivacySettings, type UsageTotal } from "../api";
import { ErrorText, Page, SettingBlock, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import GoalsSettings from "./GoalsSettings";
import SyncSettings from "./SyncSettings";
import UpdateSettings from "./UpdateSettings";

export default function Settings({ status, onChange }: { status: AppStatus; onChange: () => void }) {
  const [privacy, setPrivacy] = useState<PrivacySettings | null>(null);
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [diag, setDiag] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exported, setExported] = useState<string | null>(null);

  useEffect(() => {
    api.privacy().then(setPrivacy);
    api.knownApps().then(setApps);
  }, []);

  async function save(next: PrivacySettings) {
    try {
      setError(null);
      await api.savePrivacy(next);
      setPrivacy(next);
    } catch (e) {
      setError(String(e));
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
      <ErrorText>{error}</ErrorText>

      <SettingsGroup title="Genel">
        <ToggleRow
          label="Bilgisayar açılınca başlat"
          hint="Kum menü çubuğunda sessizce başlar."
          checked={status.autostart}
          onChange={toggleAutostart}
        />
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

      <GoalsSettings />

      {privacy && (
        <SettingsGroup title="Gizlilik" description="Değişiklikler yeni kayıtlara uygulanır; geçmiş kayıtlar değişmez.">
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

      <SettingsGroup title="Veriler">
        <SettingRow
          label="CSV olarak dışa aktar"
          hint={exported ?? "Tüm kayıtlar (başlangıç, bitiş, uygulama, başlık, kategori, proje) İndirilenler klasörüne yazılır."}
        >
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              api.exportCsv().then(
                (p) => setExported(`Kaydedildi: ${p}`),
                (e) => setError(String(e)),
              )
            }
          >
            Dışa aktar
          </Button>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Sorun giderme">
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

function Chips({ items, onRemove }: { items: { key: string; label: string; title?: string }[]; onRemove: (key: string) => void }) {
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

function SuffixEditor({ suffixes, onChange }: { suffixes: string[]; onChange: (s: string[]) => void }) {
  const [text, setText] = useState("");
  return (
    <SettingBlock
      label="Başlıklardan kaldırılacak ekler"
      hint="Bazı uygulamalar başlığın sonuna sabit bir şey ekler (örn. Firefox profil adı “— Kaan”). Buraya yazdığın ek başlıklardan silinir."
    >
      <Chips items={suffixes.map((s) => ({ key: s, label: s }))} onRemove={(s) => onChange(suffixes.filter((x) => x !== s))} />
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
