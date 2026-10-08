import { createContext, useContext, useEffect, useState } from "react";
import { Check, PenLine, Plus, Trash2, Undo2, X } from "lucide-react";
import {
  api,
  type CalendarMeeting,
  type EditScope,
  formatDuration,
  NO_PROJECT,
  type Tag,
  type WorkBlock,
} from "../api";
import { formatTime, isoDate, parseIsoDate } from "../lib/dates";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { CategorySelect } from "./CategorySelect";
import { ProjectSelect } from "./ProjectSelect";
import { friendlyError, undoable } from "../lib/feedback";
import { attendanceLine, isSkipped } from "../lib/attendance";
import { cn } from "../lib/utils";

/** "geçersiz kayıt: bu aralıkta…" → "Bu aralıkta…" */
const message = friendlyError;

/**
 * Takvimdeki blokların düzenleme bağlamı (kategoriler ve yenileme). `picked`: Shift+tıkla seçilen
 * blokların başlangıçları; Shift+A onları birleştirir.
 */
export const EditContext = createContext<{
  categories: Tag[];
  projects: Tag[];
  onChanged: () => void;
  picked?: Set<string>;
  onPick?: (b: WorkBlock) => void;
} | null>(null);

/** Seçicideki değer: kurallara bırak (elle proje yok). */
const AUTO = "__otomatik__";

/**
 * Projeye atama: işletim sisteminin açılır listesi (`ProjectSelect`). Kartın içinde ayrı katmanda
 * açılan liste Windows'ta (WebView2) seçimi kaybediyordu; yerel liste bu sorunu yaşamaz.
 * "Projesiz" kurala uysa da projeye saymaz, "Otomatik" elle atamayı kaldırıp kurallara bırakır:
 * atama her zaman geri alınabilir. Proje yoksa nereden ekleneceğini söyler.
 */
function ProjectAssign({
  value,
  projects,
  onChange,
}: {
  /** Bloğun şu anki projesi (`null`: atanmamış); aralıkta verilmez ve "Projeye ata…" görünür. */
  value?: string | null;
  projects: Tag[];
  onChange: (id: string | null) => Promise<unknown>;
}) {
  // Seçilen değer yanıt gelmeden gösterilir; hata olursa (yanıt `false`) geri alınır.
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  useEffect(() => setChosen(undefined), [value]);
  if (projects.length === 0)
    return (
      <p className="text-[11px] text-muted-foreground">
        Projeye atamak için önce kenar çubuğundaki Projeler sayfasından bir proje ekle.
      </p>
    );
  const isRange = value === undefined;
  const current = chosen ?? (isRange ? "" : (value ?? NO_PROJECT));
  return (
    <div className="space-y-1">
      <span className="block text-[11px] text-muted-foreground">Proje</span>
      <ProjectSelect
        value={current}
        projects={projects}
        placeholder={isRange ? "Projeye ata…" : null}
        extra={[
          { value: NO_PROJECT, label: "Projesiz" },
          { value: AUTO, label: "Otomatik (kurallara göre)" },
        ]}
        className="w-full"
        onChange={(v) => {
          // Otomatik seçilince değer kurallardan yenilenir; işaret yanıtla gelene bırakılır.
          setChosen(v === AUTO ? undefined : v);
          onChange(v === AUTO ? null : v).then((ok) => ok === false && setChosen(undefined));
        }}
      />
    </div>
  );
}
export const useEdit = () => useContext(EditContext);

/** Atama bildirimi: "Proje: X" ya da kurallara dönüş. */
function projectMessage(projects: Tag[], id: string | null) {
  if (id === null) return "Proje kurallara bırakıldı";
  if (id === NO_PROJECT) return "Projesiz olarak işaretlendi";
  return `${projects.find((p) => p.id === id)?.name ?? "Proje"} projesine atandı`;
}

function categoryMessage(categories: Tag[], id: string | null) {
  if (id === null) return "Kategori kurallara bırakıldı";
  return `${categories.find((c) => c.id === id)?.name ?? "Kategori"} kategorisine atandı`;
}

/** Takvimde sürükleyerek seçilen aralık ve menünün açılacağı nokta. */
/**
 * Seçilen aralık. `scope`: uygulama çizelgesindeki bir çubuktan seçildiyse yalnızca o
 * uygulamanın (ya da pencerenin) kayıtları değişir; `label` menüde adıdır.
 */
export type RangeSelection = {
  start: number;
  end: number;
  x: number;
  y: number;
  scope?: EditScope & { label: string };
};

/** İmlecin yanında açılan küçük menü; dışına tıklayınca ya da Esc ile kapanır. */
function FloatingMenu({
  x,
  y,
  label,
  onClose,
  children,
}: {
  x: number;
  y: number;
  label: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const width = 264;
  const left = Math.min(x + 12, window.innerWidth - width - 12);
  const top = Math.min(y, window.innerHeight - 240);
  return (
    <>
      <div className="fixed inset-0 z-40" onPointerDown={onClose} />
      <div
        role="dialog"
        aria-label={label}
        className="fixed z-50 space-y-3 rounded-lg border bg-popover p-3 text-popover-foreground shadow-lg"
        style={{ left, top, width }}
      >
        {children}
      </div>
    </>
  );
}

function MenuHeader({ title, sub, onClose }: { title: React.ReactNode; sub: React.ReactNode; onClose: () => void }) {
  return (
    <div className="flex items-start justify-between gap-2">
      <div className="min-w-0">
        <div className="text-sm font-medium tabular">{title}</div>
        <div className="text-xs text-muted-foreground tabular">{sub}</div>
      </div>
      <Button size="icon-sm" variant="ghost" className="-mt-1 -mr-1 shrink-0" onClick={onClose} aria-label="Kapat">
        <X />
      </Button>
    </div>
  );
}

/**
 * Seçilen aralık için menü: içindeki kayıtları kategoriye ata, elle kayıt ekle ya da sil.
 * Bırakılan noktanın yanında açılır; dışına tıklayınca ya da Esc ile kapanır.
 */
export function RangeMenu({
  selection,
  categories,
  projects,
  onAddEntry,
  onChanged,
  onClose,
}: {
  selection: RangeSelection;
  categories: Tag[];
  projects: Tag[];
  onAddEntry: (start: number, end: number) => void;
  onChanged: () => void;
  onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  // Gelecek kaydedilemez ve silinecek bir şey de yoktur: bitiş şimdiye kırpılır.
  const end = Math.min(selection.end, Date.now());
  const start = Math.min(selection.start, end);
  const future = selection.start >= Date.now();
  const iso = (t: number) => new Date(t).toISOString();
  const scoped = selection.scope;
  const scope = scoped ? { appIds: scoped.appIds, titles: scoped.titles } : null;
  const run = (f: () => Promise<unknown>) =>
    f().then(
      () => {
        onChanged();
        onClose();
      },
      (e) => {
        setError(message(e));
        return false;
      },
    );

  return (
    <FloatingMenu x={selection.x} y={selection.y} label="Seçilen aralık" onClose={onClose}>
      <MenuHeader
        title={`${formatTime(new Date(selection.start))} – ${formatTime(new Date(selection.end))}`}
        sub={
          scoped
            ? `Yalnızca ${scoped.label} · ${formatDuration((selection.end - selection.start) / 1000)}`
            : formatDuration((selection.end - selection.start) / 1000)
        }
        onClose={onClose}
      />
      {future && <p className="text-xs text-muted-foreground">Bu aralık henüz gelmedi; kayıt eklenemez.</p>}
      {end > start && (
        <ProjectAssign
          projects={projects}
          onChange={(id) =>
            run(() => undoable(api.setRangeProject(iso(start), iso(end), id, scope), projectMessage(projects, id)))
          }
        />
      )}
      {end > start && (
        <CategorySelect
          value={null}
          onChange={(id) =>
            run(() => undoable(api.setRangeCategory(iso(start), iso(end), id, scope), categoryMessage(categories, id)))
          }
          categories={categories}
          noneLabel="Kurallara göre"
          placeholder="İçindeki kayıtları kategoriye ata…"
          className="w-full"
          aria-label="Aralığın kategorisi"
        />
      )}
      {!future && (
        <div className="flex justify-end gap-2">
          {/* Elle kayıt bir uygulamaya ait değildir: yalnızca takvimden seçilen aralıkta. */}
          {!scoped && (
            <Button
              size="sm"
              variant="outline"
              className="flex-1"
              onClick={() => {
                // Bitiş şimdiye kırpılmış haliyle: gelecek kaydedilemez.
                onAddEntry(start, end);
                onClose();
              }}
            >
              <PenLine /> Elle kayıt ekle
            </Button>
          )}
          {end > start && (
            <Button
              size="icon-sm"
              variant="ghost"
              className="text-muted-foreground hover:text-destructive"
              onClick={() =>
                run(() =>
                  undoable(
                    api.deleteRange(iso(start), iso(end), scope),
                    scoped ? `${scoped.label} kayıtları silindi` : "Aralıktaki kayıtlar silindi",
                  ),
                )
              }
              aria-label={scoped ? `Aralıktaki ${scoped.label} kayıtlarını sil` : "Aralıktaki kayıtları sil"}
              title={
                scoped
                  ? `Aralıktaki ${scoped.label} kayıtlarını sil (geri alınabilir)`
                  : "Aralıktaki kayıtları sil (geri alınabilir)"
              }
            >
              <Trash2 />
            </Button>
          )}
        </div>
      )}
      {error && <p className="text-xs text-destructive selectable">{error}</p>}
    </FloatingMenu>
  );
}

/** Takvimde tıklanan toplantı ve menünün açılacağı nokta. */
export type MeetingSelection = { meeting: CalendarMeeting; x: number; y: number };

const IGNORE = "__yoksay__";

/**
 * Toplantı menüsü: toplantı serisini projeye ata (zaman çizelgesine o projeyle girer;
 * gelecekteki toplantılar da atanabilir). Toplantı sırasında bilgisayar kullanılmadığından
 * aralıkta çoğu zaman kayıt yoktur; geçmiş toplantı için elle kayıt eklemek de buradan.
 */
export function MeetingMenu({
  selection,
  projects,
  onAddEntry,
  onChanged,
  onClose,
}: {
  selection: MeetingSelection;
  projects: Tag[];
  onAddEntry: (start: number, end: number, label: string, projectId: string | null) => void;
  onChanged: () => void;
  onClose: () => void;
}) {
  const m = selection.meeting;
  const a = new Date(m.start);
  const b = new Date(m.end);
  const [value, setValue] = useState(m.projectId ?? (m.ignored ? IGNORE : ""));
  const [error, setError] = useState<string | null>(null);
  const subject = m.subject || "(konusuz)";
  // Projesi belli olmayan toplantının önerisi (seçicide olan bir projeyse).
  const suggestion = value === "" && m.suggestion ? m.suggestion : null;
  const suggestedName = suggestion ? projects.find((p) => p.id === suggestion.projectId)?.name : undefined;

  const attendance = m.attendance;
  const attendanceText = attendanceLine(m, attendance);
  // Katılım cevabı (`null`: geri al, Kum karar versin).
  function answer(attended: boolean | null) {
    if (!attendance) return;
    setError(null);
    api.answerMeeting(attendance.key, attended).then(onChanged, (e) => setError(message(e)));
  }

  function assign(v: string) {
    const before = value;
    setValue(v);
    setError(null);
    api.assignMeeting(m.uid, v === IGNORE ? null : v).then(onChanged, (e) => {
      setValue(before);
      setError(message(e));
    });
  }

  return (
    <FloatingMenu x={selection.x} y={selection.y} label="Toplantı" onClose={onClose}>
      <MenuHeader
        title={<span className="line-clamp-2 break-words">{subject}</span>}
        sub={`${formatTime(a)} – ${formatTime(b)} · ${formatDuration((+b - +a) / 1000)}`}
        onClose={onClose}
      />
      {m.agenda && (
        <p
          className="line-clamp-6 text-[11px] whitespace-pre-line break-words text-muted-foreground selectable"
          title={m.agenda}
        >
          {m.agenda}
        </p>
      )}
      {projects.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">
          Projeye atamak için önce kenar çubuğundaki Projeler sayfasından bir proje ekle.
        </p>
      ) : (
        <div className="space-y-1">
          <span className="block text-[11px] text-muted-foreground">Proje</span>
          {suggestion && suggestedName && (
            <button
              type="button"
              className="flex w-full items-center gap-1.5 rounded-md border border-dashed px-2 py-1 text-left text-xs hover:border-solid hover:bg-accent"
              title={`Öneri: ${suggestion.reason}`}
              onClick={() => assign(suggestion.projectId)}
            >
              <span className="truncate">→ {suggestedName}</span>
              <span className="ml-auto shrink-0 truncate text-[11px] text-muted-foreground">{suggestion.reason}</span>
            </button>
          )}
          <ProjectSelect
            value={value}
            projects={projects}
            placeholder={value === "" ? "Projeye ata…" : null}
            extra={[{ value: IGNORE, label: "Zaman çizelgesine alma" }]}
            className="w-full"
            aria-label="Toplantının projesi"
            onChange={assign}
          />
          <span className="block text-[11px] text-muted-foreground">
            Serinin tüm tekrarlarına uygulanır; zaman çizelgesine bu projeyle girer.
          </span>
        </div>
      )}
      {attendance && m.projectId && +a < Date.now() && (
        <div className="space-y-1">
          {attendanceText && (
            <p
              className={cn(
                "text-[11px]",
                isSkipped(attendance) ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground",
              )}
            >
              {attendanceText}
            </p>
          )}
          {attendance.auto ? (
            <Button size="sm" variant="outline" className="w-full" onClick={() => answer(isSkipped(attendance))}>
              {isSkipped(attendance) ? (
                <>
                  <Check /> Katıldım
                </>
              ) : (
                <>
                  <X /> Katılmadım
                </>
              )}
            </Button>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              className="w-full"
              title="Kum görüşmeye bakıp karar versin"
              onClick={() => answer(null)}
            >
              <Undo2 /> Cevabı geri al
            </Button>
          )}
        </div>
      )}
      {+a < Date.now() && (
        <Button
          size="sm"
          variant="outline"
          className="w-full"
          onClick={() => {
            const project = value === IGNORE ? null : value || (suggestedName && suggestion?.projectId) || null;
            onAddEntry(+a, Math.min(+b, Date.now()), m.subject, project);
            onClose();
          }}
        >
          <PenLine /> Toplantıyı elle kayıt olarak ekle
        </Button>
      )}
      {error && <p className="text-xs text-destructive selectable">{error}</p>}
    </FloatingMenu>
  );
}

/** Blok kartının altı: bloğu kategoriye ata ya da sil. */
export function BlockActions({
  start,
  end,
  categoryId,
  projectId,
  categories,
  projects,
  onChanged,
}: {
  start: string;
  end: string;
  categoryId: string | null;
  projectId: string | null;
  categories: Tag[];
  projects: Tag[];
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const run = (f: () => Promise<unknown>) =>
    f().then(onChanged, (e) => {
      setError(message(e));
      return false;
    });

  return (
    <div className="space-y-2 border-t pt-3">
      <ProjectAssign
        value={projectId}
        projects={projects}
        onChange={(id) => run(() => undoable(api.setRangeProject(start, end, id), projectMessage(projects, id)))}
      />
      <div className="flex items-center gap-2">
        <CategorySelect
          value={categoryId}
          onChange={(id) => run(() => undoable(api.setRangeCategory(start, end, id), categoryMessage(categories, id)))}
          categories={categories}
          noneLabel="Kurallara göre"
          className="min-w-0 flex-1"
          aria-label="Bloğun kategorisi"
        />
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-muted-foreground hover:text-destructive"
          onClick={() => run(() => undoable(api.deleteRange(start, end), "Blok silindi"))}
          aria-label="Bloğu sil"
          title="Bloğu sil (geri alınabilir)"
        >
          <Trash2 />
        </Button>
      </div>
      {error && <p className="text-[11px] text-destructive selectable">{error}</p>}
    </div>
  );
}

/** "Kayıt ekle": bilgisayar dışında geçen süreyi (toplantı, okuma) elle ekler. */
/** Takvimdeki boş alana tıklanınca formu o aralıkla açma isteği. */
export type EntryDraft = {
  date: string;
  from: string;
  to: string;
  seq: number;
  label?: string;
  project?: string | null;
};

export function ManualEntry({
  day,
  categories,
  projects,
  onChanged,
  draft,
  onClose,
  disabled,
}: {
  /** Varsayılan tarih (YYYY-MM-DD). */
  day: string;
  categories: Tag[];
  projects: Tag[];
  onChanged: () => void;
  draft?: EntryDraft | null;
  /** Form kapanınca (takvimdeki önizlemeyi kaldırmak için). */
  onClose?: () => void;
  /** Düğme yerinde kalır ama basılamaz (ör. ay görünümü). */
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState("");
  const [date, setDate] = useState(day);
  const [from, setFrom] = useState("09:00");
  const [to, setTo] = useState("10:00");
  const [category, setCategory] = useState<string | null>(null);
  const [project, setProject] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!draft) return;
    setDate(draft.date);
    setFrom(draft.from);
    setTo(draft.to);
    // Önceki taslağın (ör. toplantı) adı ve projesi boş alana tıklanınca taşınmasın.
    setLabel(draft.label ?? "");
    setProject(draft.project ?? null);
    setCategory(null);
    setError(null);
    setOpen(true);
  }, [draft]);

  function at(time: string) {
    const [h, m] = time.split(":").map(Number);
    const d = parseIsoDate(date);
    d.setHours(h, m, 0, 0);
    return d;
  }

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const start = at(from);
    const end = at(to);
    if (+end <= +start) return setError("Bitiş başlangıçtan sonra olmalı.");
    if (+end > Date.now()) return setError("Henüz gelmemiş bir zaman için kayıt eklenemez.");
    try {
      await undoable(
        api.addManualEntry(label, start.toISOString(), end.toISOString(), category, project),
        `“${label.trim()}” eklendi`,
      );
      setOpen(false);
      onClose?.();
      setLabel("");
      setCategory(null);
      setProject(null);
      setError(null);
      onChanged();
    } catch (err) {
      setError(message(err));
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (!o) onClose?.();
        if (o) {
          setDate(day);
          setError(null);
        }
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className="h-6 gap-1 px-2 text-xs text-muted-foreground" disabled={disabled}>
          <Plus className="size-3.5" /> Kayıt ekle
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-[22rem]">
        <form className="space-y-3" onSubmit={save}>
          <div>
            <p className="text-[13px] font-semibold">Elle kayıt</p>
            <p className="text-xs text-muted-foreground">Toplantı, okuma gibi bilgisayar dışında geçen süre.</p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="manual-label">Ne yaptın?</Label>
            <Input
              id="manual-label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="örn. Müşteri toplantısı"
              autoFocus
            />
          </div>
          <div className="grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1fr)] gap-2">
            <div className="space-y-1.5">
              <Label htmlFor="manual-date">Tarih</Label>
              <Input
                id="manual-date"
                type="date"
                value={date}
                max={isoDate(new Date())}
                onChange={(e) => setDate(e.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="manual-from">Başlangıç</Label>
              <Input
                id="manual-from"
                type="time"

                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="manual-to">Bitiş</Label>
              <Input
                id="manual-to"
                type="time"

                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </div>
          </div>
          <div className="space-y-1.5">
            <Label>Kategori</Label>
            <CategorySelect
              value={category}
              onChange={setCategory}
              categories={categories}
              noneLabel="Kategorisiz"
              className="w-full"
              aria-label="Kategori"
            />
          </div>
          {projects.length > 0 && (
            <div className="space-y-1.5">
              <Label>Proje</Label>
              <CategorySelect
                value={project}
                onChange={setProject}
                categories={projects}
                noneLabel="Atanmamış"
                className="w-full"
                aria-label="Proje"
              />
            </div>
          )}
          {error && <p className="text-xs text-destructive selectable">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                // Elle kapatmada Radix `onOpenChange` çağırmaz; önizleme de temizlensin.
                setOpen(false);
                onClose?.();
              }}
            >
              Vazgeç
            </Button>
            <Button type="submit" size="sm" disabled={!label.trim()}>
              Ekle
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}
