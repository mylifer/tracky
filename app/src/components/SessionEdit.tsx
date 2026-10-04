import { createContext, useContext, useEffect, useState } from "react";
import { ChevronDown, PenLine, Plus, Trash2, X } from "lucide-react";
import { api, type CalendarMeeting, formatDuration, NO_PROJECT, type Tag } from "../api";
import { formatTime, isoDate, parseIsoDate } from "../lib/dates";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { CategorySelect } from "./CategorySelect";
import { tagColor } from "../lib/tags";
import { friendlyError, undoable } from "../lib/feedback";

/** "geçersiz kayıt: bu aralıkta…" → "Bu aralıkta…" */
const message = friendlyError;

/** Takvimdeki blokların düzenleme bağlamı (kategoriler ve yenileme). */
export const EditContext = createContext<{ categories: Tag[]; projects: Tag[]; onChanged: () => void } | null>(null);

/** Seçicideki değerler: kurallara bırak (elle proje yok) ve seçim yapılmadı (aralık). */
const AUTO = "__otomatik__";
const PLACEHOLDER = "__sec__";

/**
 * Projeye atama: işletim sisteminin açılır listesi. Kartın içinde ayrı katmanda açılan liste
 * Windows'ta (WebView2) seçimi kaybediyordu; yerel liste bu sorunu yaşamaz. "Projesiz" kurala
 * uysa da projeye saymaz, "Otomatik" elle atamayı kaldırıp kurallara bırakır: atama her zaman
 * geri alınabilir. Proje yoksa nereden ekleneceğini söyler.
 */
function ProjectAssign({
  value,
  projects,
  onChange,
}: {
  /** Bloğun şu anki projesi (`null`: projesiz); aralıkta verilmez ve "Projeye ata…" görünür. */
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
  const current = chosen ?? (isRange ? PLACEHOLDER : (value ?? NO_PROJECT));
  const tag = projects.find((p) => p.id === current);
  return (
    <label className="block space-y-1">
      <span className="text-[11px] text-muted-foreground">Proje</span>
      <span className="relative flex items-center">
        <i
          className="pointer-events-none absolute left-2.5 size-2 rounded-full"
          style={{ background: tagColor(tag) }}
          aria-hidden
        />
        <select
          value={current}
          aria-label="Proje"
          onChange={(e) => {
            const v = e.target.value;
            if (v === PLACEHOLDER) return;
            // Otomatik seçilince değer kurallardan yenilenir; işaret yanıtla gelene bırakılır.
            setChosen(v === AUTO ? undefined : v);
            onChange(v === AUTO ? null : v).then((ok) => ok === false && setChosen(undefined));
          }}
          className="h-8 w-full appearance-none rounded-md border bg-transparent pr-7 pl-6 text-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-input/30"
        >
          {isRange && (
            <option value={PLACEHOLDER} disabled>
              Projeye ata…
            </option>
          )}
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
          <option value={NO_PROJECT}>Projesiz</option>
          <option value={AUTO}>Otomatik (kurallara göre)</option>
        </select>
        <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
      </span>
    </label>
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
export type RangeSelection = { start: number; end: number; x: number; y: number };

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
        sub={formatDuration((selection.end - selection.start) / 1000)}
        onClose={onClose}
      />
      {future && <p className="text-xs text-muted-foreground">Bu aralık henüz gelmedi; kayıt eklenemez.</p>}
      {end > start && (
        <ProjectAssign
          projects={projects}
          onChange={(id) =>
            run(() => undoable(api.setRangeProject(iso(start), iso(end), id), projectMessage(projects, id)))
          }
        />
      )}
      {end > start && (
        <CategorySelect
          value={null}
          onChange={(id) =>
            run(() => undoable(api.setRangeCategory(iso(start), iso(end), id), categoryMessage(categories, id)))
          }
          categories={categories}
          noneLabel="Kurallara göre"
          placeholder="İçindeki kayıtları kategoriye ata…"
          className="w-full"
          aria-label="Aralığın kategorisi"
        />
      )}
      {!future && (
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            className="flex-1"
            onClick={() => {
              onAddEntry(selection.start, selection.end);
              onClose();
            }}
          >
            <PenLine /> Elle kayıt ekle
          </Button>
          {end > start && (
            <Button
              size="icon-sm"
              variant="ghost"
              className="text-muted-foreground hover:text-destructive"
              onClick={() => run(() => undoable(api.deleteRange(iso(start), iso(end)), "Aralıktaki kayıtlar silindi"))}
              aria-label="Aralıktaki kayıtları sil"
              title="Aralıktaki kayıtları sil (geri alınabilir)"
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
  const [value, setValue] = useState(m.projectId ?? (m.ignored ? IGNORE : PLACEHOLDER));
  const [error, setError] = useState<string | null>(null);
  const tag = projects.find((p) => p.id === value);
  const subject = m.subject || "(konusuz)";

  function assign(v: string) {
    const before = value;
    setValue(v);
    setError(null);
    api.assignMeeting(m.uid, v === IGNORE ? null : v, isoDate(a)).then(onChanged, (e) => {
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
      {projects.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">
          Projeye atamak için önce kenar çubuğundaki Projeler sayfasından bir proje ekle.
        </p>
      ) : (
        <label className="block space-y-1">
          <span className="text-[11px] text-muted-foreground">Proje</span>
          <span className="relative flex items-center">
            <i
              className="pointer-events-none absolute left-2.5 size-2 rounded-full"
              style={{ background: tagColor(tag) }}
              aria-hidden
            />
            <select
              value={value}
              aria-label="Toplantının projesi"
              onChange={(e) => e.target.value !== PLACEHOLDER && assign(e.target.value)}
              className="h-8 w-full appearance-none rounded-md border bg-transparent pr-7 pl-6 text-xs outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 dark:bg-input/30"
            >
              {value === PLACEHOLDER && (
                <option value={PLACEHOLDER} disabled>
                  Projeye ata…
                </option>
              )}
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
              <option value={IGNORE}>Zaman çizelgesine alma</option>
            </select>
            <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
          </span>
          <span className="block text-[11px] text-muted-foreground">
            Serinin tüm tekrarlarına uygulanır; zaman çizelgesine bu projeyle girer.
          </span>
        </label>
      )}
      {+a < Date.now() && (
        <Button
          size="sm"
          variant="outline"
          className="w-full"
          onClick={() => {
            const project = value === PLACEHOLDER || value === IGNORE ? null : value;
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
}: {
  /** Varsayılan tarih (YYYY-MM-DD). */
  day: string;
  categories: Tag[];
  projects: Tag[];
  onChanged: () => void;
  draft?: EntryDraft | null;
  /** Form kapanınca (takvimdeki önizlemeyi kaldırmak için). */
  onClose?: () => void;
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
    if (draft.label !== undefined) setLabel(draft.label);
    if (draft.project !== undefined) setProject(draft.project);
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
        <Button variant="ghost" size="sm" className="h-6 gap-1 px-2 text-xs text-muted-foreground">
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
                noneLabel="Projesiz"
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
