import { useEffect, useState } from "react";
import { api, type SyncStatus } from "../api";
import { ErrorText, SettingRow, SettingsGroup } from "../components/settings";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { formatTime } from "../lib/dates";
import { useTauriEvent } from "../lib/useTauriEvent";

/** Supabase bağlantısı, giriş ve eşitleme durumu. */
export default function SyncSettings() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
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
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;

  return (
    <SettingsGroup
      title="Senkronizasyon"
      description="Verilerini kendi Supabase projende saklayarak Mac ve Windows'ta birleşik rapor görürsün. Kurulum adımları README'de."
    >
      {!status.configured ? (
        <form
          className="space-y-3 px-4 py-3.5"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => api.syncConfigure(url, key));
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
          <p className="text-xs text-muted-foreground selectable">Bağlı proje: {status.url}</p>
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
        </>
      )}
    </SettingsGroup>
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
