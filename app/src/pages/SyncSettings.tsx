import { useEffect, useState } from "react";
import { api, type KnownDevice, type SyncStatus } from "../api";
import { DeviceIcon } from "../components/DeviceIcon";
import { ErrorText, SettingBlock, SettingRow, SettingsGroup } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { formatTime } from "../lib/dates";
import { useTauriEvent } from "../lib/useTauriEvent";
import { friendlyError, toast } from "../lib/feedback";

/** Supabase bağlantısı, giriş ve eşitleme durumu. */
export default function SyncSettings() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [schema, setSchema] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.syncStatus().then(setStatus, () => {});
  }, []);
  useTauriEvent(api.onSync, setStatus);

  // Başarılı girişten sonra şifre bellekte (bileşen durumunda) tutulmasın.
  const signIn = (signUp: boolean) =>
    run(async () => {
      const s = await api.syncSignIn(email, password, signUp);
      setPassword("");
      return s;
    });

  async function run(f: () => Promise<SyncStatus | void>) {
    setBusy(true);
    setError(null);
    try {
      const s = await f();
      if (s) setStatus(s);
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;

  return (
    <SettingsGroup
      id="senkronizasyon"
      title="Senkronizasyon"
      description="Verilerini kendi Supabase projende saklayarak Mac ve Windows'ta birleşik rapor görürsün. Kurulum adımları README'de."
    >
      {!status.configured ? (
        <form
          className="space-y-3 px-4 py-3.5"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.syncConfigure(url, key, schema.trim() || null));
          }}
        >
          <Field id="sync-url" label="Proje adresi">
            <Input
              id="sync-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://abcd.supabase.co"
            />
          </Field>
          <Field id="sync-key" label="Anon (publishable) anahtar">
            <Input id="sync-key" value={key} onChange={(e) => setKey(e.target.value)} placeholder="eyJhbGciOi…" />
          </Field>
          <Field id="sync-schema" label="Şema (isteğe bağlı)">
            <Input
              id="sync-schema"
              value={schema}
              onChange={(e) => setSchema(e.target.value)}
              placeholder="Boş bırak; başka uygulamanın projesini paylaşıyorsan örn. kum"
            />
          </Field>
          <ErrorText>{error}</ErrorText>
          <Button type="submit" size="sm" disabled={busy || !url || !key}>
            Bağlantıyı kaydet
          </Button>
        </form>
      ) : !status.email ? (
        <form
          className="space-y-3 px-4 py-3.5"
          onSubmit={(e) => {
            e.preventDefault();
            signIn(false);
          }}
        >
          <p className="text-xs text-muted-foreground selectable">
            Bağlı proje: {status.url}
            {status.schema ? ` · şema: ${status.schema}` : ""}
          </p>
          <Field id="sync-email" label="E-posta">
            <Input
              id="sync-email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="username"
            />
          </Field>
          <Field id="sync-password" label="Şifre">
            <Input
              id="sync-password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          </Field>
          <ErrorText>{error}</ErrorText>
          <div className="flex items-center gap-2">
            <Button type="submit" size="sm" disabled={busy || !email || password.length < 6}>
              Giriş yap
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || !email || password.length < 6}
              onClick={() => signIn(true)}
            >
              Hesap oluştur
            </Button>
            <div className="flex-1" />
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              onClick={() => run(api.syncDisconnect)}
            >
              Bağlantıyı kaldır
            </Button>
          </div>
        </form>
      ) : (
        <>
          <SettingRow
            label={status.email}
            hint={
              status.last
                ? `${status.last.ok ? "Son eşitleme" : "Son deneme"} ${formatTime(new Date(status.last.at))} · ${status.last.message}`
                : "Henüz eşitlenmedi · her 5 dakikada bir otomatik eşitlenir"
            }
          >
            <Badge variant={status.last && !status.last.ok ? "destructive" : "success"}>
              {status.last && !status.last.ok ? "Hata" : "Bağlı"}
            </Badge>
          </SettingRow>
          <div className="flex items-center gap-2 px-4 py-2.5">
            <Button variant="outline" size="sm" disabled={busy} onClick={() => run(api.syncNow)}>
              Şimdi eşitle
            </Button>
            <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => run(api.syncSignOut)}>
              Çıkış yap
            </Button>
            <div className="flex-1" />
            <ErrorText>{error ?? (status.last && !status.last.ok ? status.last.message : null)}</ErrorText>
          </div>
          <Devices />
        </>
      )}
    </SettingsGroup>
  );
}

/** Bilgisayar adları: takvimde bloğun hangi bilgisayardan geldiği ve filtre bu adlarla görünür. */
function Devices() {
  const [devices, setDevices] = useState<KnownDevice[] | null>(null);
  useEffect(() => {
    api.listDevices().then(setDevices, () => {});
  }, []);
  useTauriEvent(api.onSync, () => {
    api.listDevices().then(setDevices, () => {});
  });
  if (!devices || devices.length === 0) return null;

  async function rename(d: KnownDevice, name: string) {
    const trimmed = name.trim();
    if (!trimmed || trimmed === d.name) return;
    try {
      setDevices(await api.renameDevice(d.id, trimmed));
      toast(`Bilgisayar adı “${trimmed}” oldu`);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  return (
    <SettingBlock
      label="Bilgisayarlar"
      hint="Takvimde bloğun hangi bilgisayardan geldiği ve bilgisayar filtresi bu adlarla görünür. Ad diğer bilgisayarlara da eşitlenir."
    >
      <ul className="max-w-md space-y-1.5">
        {devices.map((d) => (
          <li key={`${d.id}:${d.name}`} className="flex items-center gap-2">
            <DeviceIcon os={d.os} model={d.model} className="size-6 text-muted-foreground" />
            <Input
              id={`device-${d.id}`}
              className="h-8"
              defaultValue={d.name}
              aria-label="Bilgisayar adı"
              onBlur={(e) => void rename(d, e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") {
                  e.currentTarget.value = d.name;
                  e.currentTarget.blur();
                }
              }}
            />
            <span className="w-28 shrink-0 text-xs text-muted-foreground">
              {d.current
                ? "Bu bilgisayar"
                : d.model || (d.os === "windows" ? "Windows" : d.os === "macos" ? "Mac" : "")}
            </span>
          </li>
        ))}
      </ul>
    </SettingBlock>
  );
}

function Field({ id, label, children }: { id: string; label: string; children: React.ReactNode }) {
  return (
    <div className="grid max-w-md gap-1.5">
      <Label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  );
}
