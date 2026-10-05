import { useCallback, useEffect, useRef, useState } from "react";
import {
  CalendarDays,
  Check,
  Copy,
  FileSpreadsheet,
  Loader2,
  Plus,
  RefreshCw,
  Sheet,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  type AiStatus,
  type CalendarStatus,
  type ProjectMapping,
  type Tag,
  type Timesheet,
  type TimesheetConfig,
} from "../api";
import { ProjectSelect } from "../components/ProjectSelect";
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
 * Ayarlar → Bağlantılar (Outlook takvimi) ve Zaman çizelgeleri: her firmanın çizelgesi (Excel
 * dosyası ya da Google Sheets tablosu), oraya giden projeler ve birimler. Ayar burada bir kez
 * yüklenir; bir çizelge değişince diğerleri eski değeri geri yazmasın.
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
      setError(null);
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
        description="Toplantıların geldiği takvim; zaman çizelgelerinin dosyaları aşağıda."
      >
        <SettingBlock
          label="Outlook takvimi"
          hint="Toplantılar zaman çizelgesine girer: konusu bir projenin kuralına uyan toplantı o projeye yazılır."
        >
          <CalendarConnect />
        </SettingBlock>
      </SettingsGroup>
      {config && <Timesheets config={config} projects={projects} onChange={load} onError={setError} />}
      <ErrorText>{error}</ErrorText>
      <AiSettings />
    </>
  );
}

/** Firmaların zaman çizelgeleri ve adam-gün saati. */
function Timesheets({
  config,
  projects,
  onChange,
  onError,
}: {
  config: TimesheetConfig;
  projects: Tag[];
  onChange: () => void;
  onError: (e: string) => void;
}) {
  const [adding, setAdding] = useState(false);
  // Art arda iki değişiklik (yeniden yükleme bitmeden) birbirini ezmesin: her biri sonuncunun üzerine.
  const latest = useRef(config);
  useEffect(() => {
    latest.current = config;
  }, [config]);
  const save = async (next: TimesheetConfig) => {
    latest.current = next;
    try {
      await api.saveTimesheetConfig(next);
      onChange();
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  const update = (id: string, f: (t: Timesheet) => Timesheet) =>
    save({ ...latest.current, timesheets: latest.current.timesheets.map((t) => (t.id === id ? f(t) : t)) });

  return (
    <SettingsGroup
      id={TIMESHEET_SECTION}
      title="Zaman çizelgeleri"
      description="Her firmanın tablosu ayrı: bir çizelgeye yalnızca ona bağladığın projelerin işi gider. Bir proje tek bir çizelgeye bağlanır."
    >
      {config.timesheets.map((t) => (
        <TimesheetEditor
          key={sheetKey(t)}
          sheet={t}
          config={config}
          projects={projects}
          onUpdate={(f) => update(t.id, f)}
          onChange={onChange}
          onError={onError}
        />
      ))}
      <div className="space-y-3 px-4 py-3">
        {adding ? (
          <NewTimesheet
            onDone={() => {
              setAdding(false);
              onChange();
            }}
            onCancel={() => setAdding(false)}
          />
        ) : (
          <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
            <Plus /> {config.timesheets.length ? "Başka firmanın zaman çizelgesi" : "Zaman çizelgesi ekle"}
          </Button>
        )}
      </div>
      <SettingRow label="Adam-gün saati" hint="Bir adam-gün kaç saat (özetteki “ag”, sözleşme bütçeleri).">
        <Input
          className="h-8 w-20 text-sm"
          defaultValue={String(config.dayHours)}
          key={config.dayHours}
          onBlur={(e) => {
            const n = Number(e.target.value.replace(",", "."));
            if (n > 0 && n <= 24 && n !== config.dayHours) save({ ...latest.current, dayHours: n });
          }}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Adam-gün saati"
        />
      </SettingRow>
    </SettingsGroup>
  );
}

/** Dosya değişince (yeniden içe aktarma) alanlar yeni değerlerle açılsın. */
function sheetKey(t: Timesheet) {
  return `${t.id}|${t.filePath}|${t.sheetUrl}|${t.company}|${t.consultant}|${t.defaultParty}`;
}

/** Yeni firmanın zaman çizelgesi: Excel dosyası ya da Google Sheets tablosu seçilir. */
function NewTimesheet({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const [sheets, setSheets] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (sheets) return <SheetConnect timesheetId={null} onDone={onDone} onCancel={() => setSheets(false)} />;
  return (
    <div className="space-y-2">
      <p className="text-xs text-muted-foreground">
        Firmanın dosyasını seç: firma, danışman ve birimler dosyadan alınır, birimler proje olarak eklenir (yoksa).
      </p>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          onClick={async () => {
            setError(null);
            try {
              const path = await api.pickTimesheetFile();
              if (path) {
                await api.importTimesheetTemplate(null, path);
                onDone();
              }
            } catch (e) {
              setError(friendlyError(e));
            }
          }}
        >
          <FileSpreadsheet /> Excel dosyası seç
        </Button>
        <Button size="sm" variant="outline" onClick={() => setSheets(true)}>
          <Sheet /> Google Sheets'e bağla
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Vazgeç
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
    </div>
  );
}

/** Bir çizelge: dosyası, firma bilgileri, projeleri (birim, taraf, hazır açıklama) ve birimleri. */
function TimesheetEditor({
  sheet,
  config,
  projects,
  onUpdate,
  onChange,
  onError,
}: {
  sheet: Timesheet;
  config: TimesheetConfig;
  projects: Tag[];
  /** Çizelgeyi en son kaydedilen haline göre değiştirir. */
  onUpdate: (f: (t: Timesheet) => Timesheet) => void;
  onChange: () => void;
  onError: (e: string) => void;
}) {
  const [removing, setRemoving] = useState(false);
  const setMapping = (id: string, patch: Partial<ProjectMapping>) =>
    onUpdate((t) => ({ ...t, projects: t.projects.map((m) => (m.projectId === id ? { ...m, ...patch } : m)) }));
  // Başka çizelgeye bağlı olmayan projeler eklenebilir.
  const taken = new Set(config.timesheets.flatMap((t) => t.projects.map((m) => m.projectId)));
  const free = projects.filter((p) => !taken.has(p.id));
  const field = (label: string, value: string, onCommit: (v: string) => void, placeholder?: string) => (
    <label className="flex min-w-0 flex-1 flex-col gap-1 text-[11px] text-muted-foreground">
      {label}
      <Input
        className="h-8 text-sm text-foreground"
        defaultValue={value}
        placeholder={placeholder}
        onBlur={(e) => e.target.value.trim() !== value && onCommit(e.target.value.trim())}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        aria-label={label}
      />
    </label>
  );

  return (
    <div className="space-y-3 px-4 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[13px] font-semibold">{sheet.company || "Adsız çizelge"}</span>
        <span className="ml-auto" />
        {removing ? (
          <>
            <span className="text-xs text-muted-foreground">Projeleri hiçbir çizelgeye gitmez; satırlar silinmez.</span>
            <Button
              size="sm"
              variant="destructive"
              onClick={async () => {
                try {
                  await api.removeTimesheet(sheet.id);
                  onChange();
                } catch (e) {
                  onError(friendlyError(e));
                }
              }}
            >
              Çizelgeyi kaldır
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setRemoving(false)}>
              Vazgeç
            </Button>
          </>
        ) : (
          <Button size="sm" variant="ghost" onClick={() => setRemoving(true)}>
            <Trash2 /> Kaldır
          </Button>
        )}
      </div>
      <TargetSetting sheet={sheet} onChange={onChange} onError={onError} />
      <div className="flex flex-wrap gap-3">
        {field("Firma", sheet.company, (v) => onUpdate((t) => ({ ...t, company: v })))}
        {field("Danışman (“Consultant” sütunu)", sheet.consultant, (v) => onUpdate((t) => ({ ...t, consultant: v })))}
        {field("Varsayılan taraf (“Parties”)", sheet.defaultParty, (v) => onUpdate((t) => ({ ...t, defaultParty: v })))}
      </div>
      <div className="space-y-1.5">
        <div className="text-[13px] font-medium">Projeler</div>
        <p className="text-xs text-muted-foreground">
          Raporda bu projelere atadığın süre bu çizelgede satır olur. Birim, satırın birim sütununa yazılan değerdir
          (boşsa proje adı; satırda değiştirilebilir); taraf boşsa varsayılan. Hazır açıklama, pencere başlıklarından
          açıklama çıkmayan satırlara yazılır.
        </p>
        {sheet.projects.length > 0 && (
          <ul className="divide-y rounded-lg border">
            {sheet.projects.map((m) => {
              const p = projects.find((x) => x.id === m.projectId);
              return (
                <li
                  key={m.projectId}
                  className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)_110px_28px] items-center gap-2 px-3 py-1.5"
                >
                  <span className="flex min-w-0 items-center gap-2 text-[13px]">
                    <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(p) }} />
                    <span className="truncate">{p?.name ?? "Silinmiş proje"}</span>
                  </span>
                  <Input
                    className="h-7 text-xs"
                    list={`birimler-${sheet.id}`}
                    placeholder={p?.name ?? "Birim"}
                    defaultValue={m.division}
                    onBlur={(e) =>
                      e.target.value.trim() !== m.division &&
                      setMapping(m.projectId, { division: e.target.value.trim() })
                    }
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    aria-label={`${p?.name ?? "Proje"} birimi`}
                  />
                  <Input
                    className="h-7 text-xs"
                    placeholder={sheet.defaultParty || "Taraf"}
                    defaultValue={m.party ?? ""}
                    onBlur={(e) =>
                      (e.target.value.trim() || null) !== (m.party ?? null) &&
                      setMapping(m.projectId, { party: e.target.value.trim() || null })
                    }
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    aria-label={`${p?.name ?? "Proje"} tarafı`}
                  />
                  <button
                    className="grid size-7 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-destructive"
                    aria-label={`${p?.name ?? "Projeyi"} bu çizelgeden çıkar`}
                    title="Bu çizelgeden çıkar: işi artık buraya gitmez"
                    onClick={() =>
                      onUpdate((t) => ({ ...t, projects: t.projects.filter((x) => x.projectId !== m.projectId) }))
                    }
                  >
                    <X className="size-3.5" />
                  </button>
                  <Input
                    className="col-span-2 col-start-2 h-7 text-xs"
                    placeholder="Hazır açıklama (isteğe bağlı)"
                    defaultValue={m.defaultDetails ?? ""}
                    onBlur={(e) => {
                      const v = e.target.value.trim() || null;
                      if (v !== (m.defaultDetails ?? null)) setMapping(m.projectId, { defaultDetails: v });
                    }}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    aria-label={`${p?.name ?? "Proje"} hazır açıklaması`}
                  />
                </li>
              );
            })}
          </ul>
        )}
        {free.length > 0 ? (
          <ProjectSelect
            value=""
            projects={free}
            placeholder="Proje ekle…"
            className="w-60"
            aria-label={`${sheet.company || "Çizelgeye"} proje ekle`}
            onChange={(projectId) =>
              onUpdate((t) => ({
                ...t,
                projects: [...t.projects, { projectId, division: "", party: null, defaultDetails: null }],
              }))
            }
          />
        ) : (
          sheet.projects.length === 0 && (
            <p className="text-xs text-muted-foreground">Boşta proje yok (kenar çubuğunda Projeler).</p>
          )
        )}
      </div>
      <Divisions sheet={sheet} onUpdate={onUpdate} />
    </div>
  );
}

/** Dosyadaki birimler: satırın birimi bunlardan seçilir. */
function Divisions({ sheet, onUpdate }: { sheet: Timesheet; onUpdate: (f: (t: Timesheet) => Timesheet) => void }) {
  const [word, setWord] = useState("");
  const add = () => {
    const d = word.trim();
    setWord("");
    if (d && !sheet.divisions.some((x) => x.toLocaleLowerCase("tr") === d.toLocaleLowerCase("tr")))
      onUpdate((t) => ({ ...t, divisions: [...t.divisions, d] }));
  };
  return (
    <div className="space-y-1.5">
      <div className="text-[13px] font-medium">Birimler</div>
      <p className="text-xs text-muted-foreground">
        Tablonun birim sütunundaki değerler (dosyadan alınır). Tek projeyle çalışıyorsan her satırın birimini bunlardan
        seçersin.
      </p>
      <datalist id={`birimler-${sheet.id}`}>
        {sheet.divisions.map((d) => (
          <option key={d} value={d} />
        ))}
      </datalist>
      <ul className="flex flex-wrap gap-1.5">
        {sheet.divisions.map((d) => (
          <li
            key={d}
            className="inline-flex items-center gap-1 rounded-full border bg-card py-0.5 pr-0.5 pl-2.5 text-xs"
          >
            {d}
            <button
              className="grid size-5 place-items-center rounded-full text-muted-foreground hover:bg-accent hover:text-foreground"
              aria-label={`${d} birimini kaldır`}
              onClick={() => onUpdate((t) => ({ ...t, divisions: t.divisions.filter((x) => x !== d) }))}
            >
              <X className="size-3" />
            </button>
          </li>
        ))}
        <li>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              add();
            }}
          >
            <Input
              className="h-6 w-36 text-xs"
              placeholder="Birim ekle…"
              value={word}
              onChange={(e) => setWord(e.target.value)}
              onBlur={add}
              aria-label="Birim ekle"
            />
          </form>
        </li>
      </ul>
    </div>
  );
}

/** Çizelgenin kayıtlarının yazılacağı yer: Excel dosyası ya da Google Sheets. */
function TargetSetting({
  sheet,
  onChange,
  onError,
}: {
  sheet: Timesheet;
  onChange: () => void;
  onError: (e: string) => void;
}) {
  const [sheetSetup, setSheetSetup] = useState(false);
  const pickExcel = async () => {
    try {
      const path = await api.pickTimesheetFile();
      if (path) {
        await api.importTimesheetTemplate(sheet.id, path);
        onChange();
      }
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  const current = sheet.sheetUrl
    ? { icon: <Sheet className="size-3.5" />, label: "Google Sheets", value: sheet.sheetLink ?? sheet.sheetUrl }
    : sheet.filePath
      ? { icon: <FileSpreadsheet className="size-3.5" />, label: "Excel", value: sheet.filePath }
      : null;

  return (
    <div className="space-y-2">
      {current ? (
        <div className="flex items-center gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-xs">
          {current.icon}
          <span className="text-muted-foreground">{current.label}</span>
          <span className="min-w-0 flex-1 truncate font-medium selectable" title={current.value}>
            {sheet.sheetUrl ? current.value : fileName(current.value)}
          </span>
        </div>
      ) : (
        <p className="text-xs text-amber-700 dark:text-amber-400">Kayıtların yazılacağı dosya seçilmedi.</p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={pickExcel}>
          <FileSpreadsheet /> {sheet.filePath && !sheet.sheetUrl ? "Başka Excel dosyası seç" : "Excel dosyası seç"}
        </Button>
        <Button size="sm" variant="outline" onClick={() => setSheetSetup((v) => !v)} aria-expanded={sheetSetup}>
          <Sheet /> {sheet.sheetUrl ? "Sheets betiği / adresi" : "Google Sheets'e bağla"}
        </Button>
        {sheet.sheetUrl && (
          <Button
            size="sm"
            variant="ghost"
            onClick={async () => {
              try {
                await api.disconnectSheet(sheet.id);
                onChange();
              } catch (e) {
                onError(friendlyError(e));
              }
            }}
            title={sheet.filePath ? `Kayıtlar yeniden ${sheet.filePath} dosyasına gider` : undefined}
          >
            {sheet.filePath ? "Excel'e dön" : "Sheets bağlantısını kaldır"}
          </Button>
        )}
      </div>
      {sheetSetup && (
        <div className="rounded-lg border px-3 py-3">
          <SheetConnect
            timesheetId={sheet.id}
            initial={{ url: sheet.sheetUrl, link: sheet.sheetLink }}
            onDone={() => {
              setSheetSetup(false);
              onChange();
            }}
            onCancel={() => setSheetSetup(false)}
          />
        </div>
      )}
    </div>
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

/**
 * Yapay zekâyla açıklama yazma: isteğe bağlı, varsayılan kapalı. Kullanıcının kendi Anthropic API
 * anahtarı ayarlarda saklanır (senkronizasyon açıksa kendi Supabase projene eşitlenir); istek
 * yalnızca zaman çizelgesindeki düğmeyle gider.
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
            : "console.anthropic.com → API Keys'ten oluştur. Ayarlarda saklanır; senkronizasyon açıksa kendi Supabase projene eşitlenir."
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
