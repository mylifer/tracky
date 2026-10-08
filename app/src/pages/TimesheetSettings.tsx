import { useCallback, useEffect, useRef, useState } from "react";
import { FileSpreadsheet, Plus, Sheet, Trash2, X } from "lucide-react";
import { api, type ProjectMapping, type Tag, type Timesheet, type TimesheetConfig } from "../api";
import { ProjectSelect } from "../components/ProjectSelect";
import { ErrorText, SettingBlock, SettingRow, SettingsGroup } from "../components/settings";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { tagColor } from "../lib/tags";
import { friendlyError } from "../lib/feedback";
import { AiSettings } from "./AiSettings";
import { CONNECTIONS_SECTION, TIMESHEET_SECTION } from "./settingsSections";
import { CalendarConnect, GoogleConnect, SheetConnect } from "./ConnectionSettings";

export { AI_SECTION, CONNECTIONS_SECTION, TIMESHEET_SECTION } from "./settingsSections";

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
        <SettingBlock
          label="Google hesabı"
          hint="Bağlıysa zaman çizelgesi tabloları Google Sheets API ile doğrudan okunup yazılır: Apps Script'e göre çok daha hızlı (saniyeler değil, yarım saniyenin altında). Çizelgenin tablo bağlantısı (docs.google.com/…) girilmiş olmalı."
        >
          <GoogleConnect />
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
    const prev = latest.current;
    latest.current = next;
    try {
      await api.saveTimesheetConfig(next);
      onChange();
    } catch (e) {
      // Kaydedilemeyen değişiklik sonrakinin temeli olmasın (arada yenisi gelmediyse).
      if (latest.current === next) latest.current = prev;
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
