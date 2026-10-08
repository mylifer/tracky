import { useEffect, useState } from "react";
import { CalendarDays, Check, Copy, Loader2, RefreshCw, Sheet } from "lucide-react";
import { api, type GoogleStatus, type CalendarStatus } from "../api";
import { ErrorText } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { useTauriEvent } from "../lib/useTauriEvent";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";

const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });

/** Outlook takviminin yayımlanan ICS bağlantısı: bağla, yenile, kaldır. */
export function CalendarConnect() {
  const [status, setStatus] = useState<CalendarStatus | null>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.calendarStatus().then(
      (s) => {
        setStatus(s);
        setUrl(s.url ?? "");
      },
      (e) => setError(friendlyError(e)),
    );
  }, []);
  useTauriEvent(api.onCalendar, setStatus);
  const save = async (next: string | null) => {
    setBusy(true);
    setError(null);
    try {
      const s = await api.setCalendarUrl(next);
      setStatus(s);
      setUrl(s.url ?? "");
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  };
  const last = status?.last;
  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          className="h-8 min-w-0 flex-1 text-xs"
          placeholder="https://outlook.office365.com/owa/calendar/…/calendar.ics"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          aria-label="Outlook takviminin ICS bağlantısı"
        />
        <Button size="sm" disabled={busy || !url.trim() || url === status?.url} onClick={() => save(url)}>
          {busy ? "Okunuyor…" : status?.url ? "Değiştir" : "Bağla"}
        </Button>
        {status?.url && (
          <>
            <Button
              size="sm"
              variant="ghost"
              aria-label="Takvimi yenile"
              onClick={() => {
                setError(null);
                api.refreshCalendar().catch((e) => setError(friendlyError(e)));
              }}
            >
              <RefreshCw />
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => save(null)}>
              Kaldır
            </Button>
          </>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
      {status?.url ? (
        <p className={cn("text-[11px]", last && !last.ok ? "text-destructive" : "text-muted-foreground")}>
          <CalendarDays className="mr-1 inline size-3 align-[-2px]" />
          {last
            ? `${last.ok ? "Okundu" : "Okunamadı"} ${timeFmt.format(new Date(last.at))}: ${last.message}`
            : `${status.events} etkinlik`}
          {" · 15 dakikada bir yenilenir. Projesi bulunamayan toplantılar zaman çizelgesinde gün kartında atanır."}
          {status.ignored > 0 && (
            <>
              {" "}
              {status.ignored} toplantı yoksayıldı ·{" "}
              <button
                className="underline underline-offset-2"
                onClick={async () => {
                  setError(null);
                  try {
                    setStatus(await api.restoreIgnoredMeetings());
                  } catch (e) {
                    setError(friendlyError(e));
                  }
                }}
              >
                geri getir
              </button>
            </>
          )}
        </p>
      ) : (
        <p className="text-[11px] text-muted-foreground">
          Outlook web'de <b>Ayarlar → Takvim → Paylaşılan takvimler → Takvim yayımla</b>: takvimi seç, "Tüm ayrıntıları
          görebilir", Yayımla; çıkan <b>ICS</b> bağlantısını buraya yapıştır. Bağlantıyı bilen herkes takvimi görebilir;
          kimseyle paylaşma.
        </p>
      )}
    </div>
  );
}

/**
 * Google hesabı: kullanıcının kendi Google Cloud projesindeki "Masaüstü uygulaması" OAuth
 * istemcisiyle tarayıcıda giriş. Bağlantı yalnızca bu cihazda saklanır.
 */
export function GoogleConnect() {
  const [status, setStatus] = useState<GoogleStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.googleStatus().then(
      (s) => {
        setStatus(s);
        setClientId(s.clientId);
      },
      (e) => setError(friendlyError(e)),
    );
  }, []);
  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.googleConnect(clientId, secret));
      setSecret("");
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  };
  if (!status) return <ErrorText>{error}</ErrorText>;
  if (status.connected)
    return (
      <div className="space-y-1.5">
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <Check className="size-3.5 text-success" />
          <span className="min-w-0 flex-1">
            Bağlı{status.email ? `: ${status.email}` : ""} · tablolar doğrudan okunup yazılıyor
          </span>
          <Button
            size="sm"
            variant="ghost"
            onClick={async () => {
              try {
                setStatus(await api.googleDisconnect());
              } catch (e) {
                setError(friendlyError(e));
              }
            }}
          >
            Bağlantıyı kes
          </Button>
        </div>
        <ErrorText>{error}</ErrorText>
      </div>
    );
  return (
    <div className="space-y-2">
      <details className="rounded-md border bg-muted/30 px-3 py-2 text-xs" open={!status.clientId}>
        <summary className="cursor-pointer font-medium">Kurulum (bir kez, ~5 dakika)</summary>
        <ol className="mt-2 list-decimal space-y-1.5 pl-4 text-muted-foreground">
          <li>
            <b>console.cloud.google.com</b>'da yeni bir proje aç (örn. "Kum").
          </li>
          <li>
            <b>APIs &amp; Services → Library</b>'de <b>Google Sheets API</b>'yi bul, <b>Enable</b>.
          </li>
          <li>
            <b>Google Auth Platform → Get started</b>: uygulama adı "Kum", e-postan, kullanıcı türü <b>External</b>.
            Sonra <b>Audience → Publish app</b> (yayınlamazsan bağlantı 7 günde bir düşer).
          </li>
          <li>
            <b>Clients → Create client</b>: tür <b>Desktop app</b>. Çıkan <b>Client ID</b> ve <b>Client secret</b>'ı
            aşağıya yapıştır.
          </li>
          <li>
            <b>Google ile bağlan</b>: tarayıcıda tablonun erişimi olan hesabı seç. "Google bu uygulamayı doğrulamadı"
            uyarısında <b>Gelişmiş → Kum'a git</b> (uygulama senin projen).
          </li>
        </ol>
      </details>
      <div className="grid gap-2 sm:grid-cols-[1fr_1fr_auto]">
        <Input
          className="h-8 text-xs"
          placeholder="Client ID (…apps.googleusercontent.com)"
          value={clientId}
          onChange={(e) => setClientId(e.target.value)}
          aria-label="OAuth istemci kimliği"
        />
        <Input
          className="h-8 text-xs"
          type="password"
          placeholder={status.hasSecret ? "Client secret (kayıtlı)" : "Client secret"}
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          aria-label="OAuth istemcisinin gizli anahtarı"
        />
        {busy ? (
          <Button size="sm" variant="outline" onClick={() => api.googleCancel()}>
            <Loader2 className="animate-spin" /> İptal
          </Button>
        ) : (
          <Button size="sm" disabled={!clientId.trim() || (!secret.trim() && !status.hasSecret)} onClick={connect}>
            Google ile bağlan
          </Button>
        )}
      </div>
      {busy && <p className="text-[11px] text-muted-foreground">Tarayıcıda Google girişini tamamla…</p>}
      <ErrorText>{error}</ErrorText>
    </div>
  );
}

/**
 * Google Sheets bağlantısı: tabloya Kum'un Apps Script'i eklenir ve web uygulaması olarak
 * dağıtılır; Kum kayıtları o adrese gönderir (Google Cloud projesi ya da giriş gerekmez).
 */
export function SheetConnect({
  timesheetId,
  onDone,
  onCancel,
  initial,
}: {
  /** Bağlanan çizelge; `null` ise yeni çizelge. */
  timesheetId: string | null;
  onDone: () => void;
  onCancel?: () => void;
  initial?: { url: string | null; link: string | null };
}) {
  const [script, setScript] = useState("");
  const [link, setLink] = useState(initial?.link ?? "");
  const [url, setUrl] = useState(initial?.url ?? "");
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.sheetScript().then(setScript, (e) => setError(friendlyError(e)));
  }, []);
  const step = "flex gap-2.5 text-[13px]";
  const num = "flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-[11px] font-semibold";
  return (
    <div className="space-y-3">
      <ol className="space-y-3">
        <li className={step}>
          <span className={num}>1</span>
          <div className="min-w-0 flex-1 space-y-1.5">
            <div>Tablonun bağlantısı (kayıtlar ilk sayfaya eklenir):</div>
            <Input
              className="h-8 text-xs"
              placeholder="https://docs.google.com/spreadsheets/d/…"
              value={link}
              onChange={(e) => setLink(e.target.value)}
            />
          </div>
        </li>
        <li className={step}>
          <span className={num}>2</span>
          <div className="min-w-0 flex-1 space-y-1.5">
            <div>
              Tabloda <b>Uzantılar → Apps Script</b>'i aç, içindekini silip bu betiği yapıştır ve kaydet.
            </div>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={!script}
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(script);
                    setCopied(true);
                  } catch {
                    setError("Kopyalanamadı; aşağıdaki kutudan seçip kopyala.");
                  }
                }}
              >
                {copied ? <Check /> : <Copy />} {copied ? "Kopyalandı" : "Betiği kopyala"}
              </Button>
            </div>
            <textarea
              readOnly
              value={script}
              onFocus={(e) => e.currentTarget.select()}
              className="h-20 w-full resize-none rounded-md border bg-muted/40 p-2 font-mono text-[10px] text-muted-foreground selectable"
              aria-label="Apps Script betiği"
            />
          </div>
        </li>
        <li className={step}>
          <span className={num}>3</span>
          <div className="min-w-0 flex-1">
            <b>Dağıt → Yeni dağıtım → Web uygulaması</b>: "Şu kullanıcı olarak yürüt: <b>Ben</b>", "Erişimi olanlar:{" "}
            <b>Herkes</b>". İstenen yetkiyi ver (Gelişmiş → güvenli olmayan sayfaya git: betik senin, yalnızca bu
            tabloya erişir).
          </div>
        </li>
        <li className={step}>
          <span className={num}>4</span>
          <div className="min-w-0 flex-1 space-y-1.5">
            <div>Çıkan web uygulaması adresini yapıştır:</div>
            <Input
              className="h-8 text-xs"
              placeholder="https://script.google.com/macros/s/…/exec"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </div>
        </li>
      </ol>
      <div className="flex gap-2">
        <Button
          disabled={busy || !url.trim()}
          onClick={async () => {
            setBusy(true);
            setError(null);
            try {
              await api.connectSheet(timesheetId, url.trim(), link.trim() || null);
              onDone();
            } catch (e) {
              setError(friendlyError(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <Sheet /> {busy ? "Bağlanıyor…" : "Bağlan ve içe aktar"}
        </Button>
        {onCancel && (
          <Button variant="ghost" onClick={onCancel}>
            Vazgeç
          </Button>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
      <p className="text-xs text-muted-foreground">
        Betik yalnızca Kum'un anahtarını taşıyan istekleri kabul eder ve aynı kaydı iki kez yazmaz. Tablo firmanın
        hesabındaysa Apps Script ya da "Herkes" erişimi kapatılmış olabilir; o zaman Excel dosyasını kullan.
      </p>
    </div>
  );
}
