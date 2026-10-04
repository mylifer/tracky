import { useCallback, useEffect, useState } from "react";
import { CalendarDays, Check, Copy, FileSpreadsheet, Loader2, RefreshCw, Sheet, Sparkles } from "lucide-react";
import { api, type AiStatus, type CalendarStatus, type Tag, type TimesheetConfig } from "../api";
import { ErrorText, SettingBlock, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { tagColor } from "../lib/tags";
import { useTauriEvent } from "../lib/useTauriEvent";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";

/** Ayarlar sayfasındaki bölüm kimlikleri (başka sayfalardan doğrudan açmak için). */
export const CONNECTIONS_SECTION = "baglantilar";
export const TIMESHEET_SECTION = "zaman-cizelgesi";
export const AI_SECTION = "yapay-zeka";

const timeFmt = new Intl.DateTimeFormat("tr-TR", { hour: "2-digit", minute: "2-digit" });

/** Dosya yolunun son parçası. */
export function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

/**
 * Ayarlar → Bağlantılar (Outlook takvimi, kayıtların yazılacağı Excel / Google Sheets) ve
 * Zaman çizelgesi (firma bilgileri, proje → birim). İkisi aynı ayarı düzenlediği için ayar
 * burada bir kez yüklenir; biri değişince diğeri eski değeri geri yazmasın.
 */
export function TimesheetSections() {
  const [config, setConfig] = useState<TimesheetConfig | null>(null);
  const [projects, setProjects] = useState<Tag[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      const [c, tax] = await Promise.all([api.timesheetConfig(), api.taxonomy()]);
      setConfig(c);
      setProjects(tax.tags.filter((t) => t.kind === "project"));
    } catch (e) {
      setError(friendlyError(e));
    }
  }, []);
  useEffect(() => {
    load();
  }, [load]);

  return (
    <>
      <SettingsGroup
        id={CONNECTIONS_SECTION}
        title="Bağlantılar"
        description="Toplantıların geldiği takvim ve zaman çizelgesi kayıtlarının yazıldığı dosya."
      >
        <SettingBlock
          label="Outlook takvimi"
          hint="Toplantılar zaman çizelgesine girer: konusu bir projenin kuralına uyan toplantı o projeye yazılır."
        >
          <CalendarConnect />
        </SettingBlock>
        {config && <TargetSetting config={config} onChange={load} onError={setError} />}
      </SettingsGroup>
      {config && (
        <TimesheetDetails
          config={config}
          projects={projects}
          onSaved={load}
          onError={setError}
          key={configKey(config)}
        />
      )}
      <ErrorText>{error}</ErrorText>
      <AiSettings />
    </>
  );
}

/** Dosya değişince (yeniden içe aktarma) alanlar yeni değerlerle açılsın. */
function configKey(c: TimesheetConfig) {
  return `${c.filePath}|${c.sheetUrl}|${c.company}|${c.consultant}|${c.defaultParty}|${c.dayHours}`;
}

/** Kayıtların yazılacağı yer: Excel dosyası ya da Google Sheets. */
function TargetSetting({
  config,
  onChange,
  onError,
}: {
  config: TimesheetConfig;
  onChange: () => void;
  onError: (e: string) => void;
}) {
  const [sheetSetup, setSheetSetup] = useState(false);
  const pickExcel = async () => {
    try {
      const path = await api.pickTimesheetFile();
      if (path) {
        await api.importTimesheetTemplate(path);
        onChange();
      }
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  const current = config.sheetUrl
    ? { icon: <Sheet className="size-3.5" />, label: "Google Sheets", value: config.sheetLink ?? config.sheetUrl }
    : config.filePath
      ? { icon: <FileSpreadsheet className="size-3.5" />, label: "Excel", value: config.filePath }
      : null;

  return (
    <SettingBlock
      label="Zaman çizelgesi dosyası"
      hint="Onaylanan kayıtlar buraya, firmanın sütun ve biçimiyle eklenir. Seçince firma, danışman, birimler ve geçmiş açıklamalar dosyadan alınır."
    >
      {current ? (
        <div className="flex items-center gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-xs">
          {current.icon}
          <span className="text-muted-foreground">{current.label}</span>
          <span className="min-w-0 flex-1 truncate font-medium selectable" title={current.value}>
            {config.sheetUrl ? current.value : fileName(current.value)}
          </span>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">Henüz seçilmedi.</p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={pickExcel}>
          <FileSpreadsheet /> {config.filePath && !config.sheetUrl ? "Başka Excel dosyası seç" : "Excel dosyası seç"}
        </Button>
        <Button size="sm" variant="outline" onClick={() => setSheetSetup((v) => !v)} aria-expanded={sheetSetup}>
          <Sheet /> {config.sheetUrl ? "Sheets betiği / adresi" : "Google Sheets'e bağla"}
        </Button>
        {config.sheetUrl && (
          <Button
            size="sm"
            variant="ghost"
            onClick={async () => {
              try {
                await api.disconnectSheet();
                onChange();
              } catch (e) {
                onError(friendlyError(e));
              }
            }}
            title={config.filePath ? `Kayıtlar yeniden ${config.filePath} dosyasına gider` : undefined}
          >
            {config.filePath ? "Excel'e dön" : "Sheets bağlantısını kaldır"}
          </Button>
        )}
      </div>
      {sheetSetup && (
        <div className="rounded-lg border px-3 py-3">
          <SheetConnect
            initial={{ url: config.sheetUrl, link: config.sheetLink }}
            onDone={() => {
              setSheetSetup(false);
              onChange();
            }}
            onCancel={() => setSheetSetup(false)}
          />
        </div>
      )}
    </SettingBlock>
  );
}

/** Firma bilgileri ve proje → birim eşlemesi. */
function TimesheetDetails({
  config,
  projects,
  onSaved,
  onError,
}: {
  config: TimesheetConfig;
  projects: Tag[];
  onSaved: () => void;
  onError: (e: string) => void;
}) {
  const [c, setC] = useState(config);
  useEffect(() => setC(config), [config]);
  const save = async (next: TimesheetConfig) => {
    setC(next);
    try {
      await api.saveTimesheetConfig(next);
      onSaved();
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  const mapping = (id: string) => c.projects.find((m) => m.projectId === id);
  const setMapping = (
    id: string,
    patch: { division?: string; party?: string | null; defaultDetails?: string | null },
  ) => {
    const current = mapping(id) ?? {
      projectId: id,
      division: projects.find((p) => p.id === id)?.name ?? "",
      party: null,
    };
    save({ ...c, projects: [...c.projects.filter((m) => m.projectId !== id), { ...current, ...patch }] });
  };
  const field = (label: string, hint: string, value: string, onCommit: (v: string) => void, width = "w-48") => (
    <SettingRow label={label} hint={hint}>
      <Input
        className={cn("h-8 text-sm", width)}
        defaultValue={value}
        onBlur={(e) => e.target.value.trim() !== value && onCommit(e.target.value.trim())}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        aria-label={label}
      />
    </SettingRow>
  );

  return (
    <SettingsGroup
      id={TIMESHEET_SECTION}
      title="Zaman çizelgesi"
      description="Dosyadan içe aktarılır; buradan düzeltebilirsin."
    >
      {field("Firma", "Raporun gittiği firma.", c.company, (v) => save({ ...c, company: v }))}
      {field("Danışman", "“Consultant” sütununa yazılan ad.", c.consultant, (v) => save({ ...c, consultant: v }))}
      {field("Varsayılan taraf", "“Parties” sütunu; projede ayrıca belirtilmediyse.", c.defaultParty, (v) =>
        save({ ...c, defaultParty: v }),
      )}
      {field(
        "Adam-gün saati",
        "Bir adam-gün kaç saat (özetteki “ag”).",
        String(c.dayHours),
        (v) => {
          const n = Number(v.replace(",", "."));
          if (n > 0 && n <= 24) save({ ...c, dayHours: n });
        },
        "w-20",
      )}
      <SettingBlock
        label="Proje → birim, taraf ve hazır açıklama"
        hint="Projenin dosyadaki adı (birim sütunu) ve tarafı; boş taraf varsayılanı kullanır. Hazır açıklama, pencere başlıklarından açıklama çıkmayan önerilere (örn. toplantı uygulaması) yazılır."
      >
        {projects.length === 0 ? (
          <p className="text-xs text-muted-foreground">Henüz proje yok (kenar çubuğunda Projeler).</p>
        ) : (
          <ul className="divide-y rounded-lg border">
            {projects.map((p) => (
              <li
                key={p.id}
                className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_120px] items-center gap-2 px-3 py-1.5"
              >
                <span className="flex min-w-0 items-center gap-2 text-[13px]">
                  <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
                  <span className="truncate">{p.name}</span>
                </span>
                <Input
                  className="h-7 text-xs"
                  defaultValue={mapping(p.id)?.division ?? p.name}
                  onBlur={(e) => setMapping(p.id, { division: e.target.value.trim() })}
                  aria-label={`${p.name} birimi`}
                />
                <Input
                  className="h-7 text-xs"
                  placeholder={c.defaultParty}
                  defaultValue={mapping(p.id)?.party ?? ""}
                  onBlur={(e) => setMapping(p.id, { party: e.target.value.trim() || null })}
                  aria-label={`${p.name} tarafı`}
                />
                <Input
                  className="col-span-2 col-start-2 h-7 text-xs"
                  placeholder="Hazır açıklama (isteğe bağlı)"
                  defaultValue={mapping(p.id)?.defaultDetails ?? ""}
                  onBlur={(e) => {
                    const v = e.target.value.trim() || null;
                    if (v !== (mapping(p.id)?.defaultDetails ?? null)) setMapping(p.id, { defaultDetails: v });
                  }}
                  onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  aria-label={`${p.name} hazır açıklaması`}
                />
              </li>
            ))}
          </ul>
        )}
      </SettingBlock>
    </SettingsGroup>
  );
}

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
            <Button size="sm" variant="ghost" aria-label="Takvimi yenile" onClick={() => api.refreshCalendar()}>
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
                onClick={async () => setStatus(await api.restoreIgnoredMeetings())}
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
 * Google Sheets bağlantısı: tabloya Kum'un Apps Script'i eklenir ve web uygulaması olarak
 * dağıtılır; Kum kayıtları o adrese gönderir (Google Cloud projesi ya da giriş gerekmez).
 */
export function SheetConnect({
  onDone,
  onCancel,
  initial,
}: {
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
              await api.connectSheet(url.trim(), link.trim() || null);
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

/**
 * Yapay zekâyla açıklama yazma: isteğe bağlı, varsayılan kapalı. Kullanıcının kendi Anthropic API
 * anahtarı bu cihazda saklanır (eşitlenmez); istek yalnızca zaman çizelgesindeki düğmeyle gider.
 */
function AiSettings() {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    api.aiSettings().then(setStatus, (e) => setResult({ ok: false, text: friendlyError(e) }));
  }, []);
  if (!status) return result ? <ErrorText>{result.text}</ErrorText> : null;

  const save = async (enabled: boolean, apiKey?: string) => {
    try {
      setStatus(await api.saveAiSettings(enabled, apiKey));
      if (apiKey !== undefined) setKey("");
      setResult(null);
    } catch (e) {
      setResult({ ok: false, text: friendlyError(e) });
    }
  };
  const test = async () => {
    setBusy(true);
    setResult(null);
    try {
      setResult({ ok: true, text: await api.testAi(key.trim() || undefined) });
    } catch (e) {
      setResult({ ok: false, text: friendlyError(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsGroup
      id={AI_SECTION}
      title="Yapay zekâ"
      description="Zaman çizelgesi açıklamalarını Claude'a yazdır. İsteğe bağlı; kendi Anthropic API anahtarınla çalışır."
    >
      <ToggleRow
        label="Yapay zekâyla yaz"
        hint="Gün kartında ve dönem denetiminde “Yapay zekâyla yaz” düğmesi görünür."
        checked={status.enabled}
        onChange={(v) => save(v)}
      />
      <SettingBlock
        label="Anthropic API anahtarı"
        hint={
          status.hasKey
            ? `Kayıtlı (${status.keyHint ?? "…"}). Değiştirmek için yenisini yaz.`
            : "console.anthropic.com → API Keys'ten oluştur. Yalnızca bu bilgisayarda saklanır, eşitlenmez."
        }
      >
        <form
          className="flex flex-wrap gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (key.trim()) save(status.enabled, key.trim());
          }}
        >
          <Input
            type="password"
            className="h-8 w-72 text-sm"
            placeholder={status.hasKey ? "••••••••" : "sk-ant-…"}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            aria-label="Anthropic API anahtarı"
          />
          <Button type="submit" size="sm" variant="outline" disabled={!key.trim()}>
            Kaydet
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || (!key.trim() && !status.hasKey)}
            onClick={test}
          >
            {busy ? <Loader2 className="animate-spin" /> : <Sparkles />} Bağlantıyı dene
          </Button>
          {status.hasKey && (
            <Button type="button" size="sm" variant="ghost" onClick={() => save(false, "")}>
              Anahtarı sil
            </Button>
          )}
        </form>
        {result && (
          <p className={cn("text-xs selectable", result.ok ? "text-success" : "text-destructive")}>{result.text}</p>
        )}
        <div className="rounded-lg border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <b className="font-medium text-foreground">Gizlilik.</b> Hiçbir şey kendiliğinden gönderilmez. Yalnızca “Yapay
          zekâyla yaz” düğmesine bastığında, yazılacak satırlar için Anthropic'e (api.anthropic.com) şunlar gider: proje
          ve müşteri adı, tür, saat ve başlangıç; o satırın süresindeki pencere başlıkları, iş anahtarları ve site
          adları ya da toplantı konusu; üslup örneği olarak o projelere daha önce yazdığın en çok 10'ar açıklama
          (projede hiç yoksa diğer projelerinden 5). Gün başına bir istek yapılır; ücreti API anahtarının hesabından
          düşer ({status.model}).
        </div>
      </SettingBlock>
    </SettingsGroup>
  );
}
