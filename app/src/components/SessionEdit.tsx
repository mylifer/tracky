import { createContext, useContext, useEffect, useState } from "react";
import { PenLine, Plus, Trash2, X } from "lucide-react";
import { api, formatDuration, type Tag } from "../api";
import { formatTime, isoDate, parseIsoDate } from "../lib/dates";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { CategorySelect } from "./CategorySelect";

/** "geçersiz kayıt: bu aralıkta…" → "Bu aralıkta…" */
function message(e: unknown) {
  const m = String(e).replace(/^geçersiz kayıt: /, "");
  return m.charAt(0).toLocaleUpperCase("tr") + m.slice(1);
}

/** Takvimdeki blokların düzenleme bağlamı (kategoriler ve yenileme). */
export const EditContext = createContext<{ categories: Tag[]; onChanged: () => void } | null>(null);
export const useEdit = () => useContext(EditContext);

/** Takvimde sürükleyerek seçilen aralık ve menünün açılacağı nokta. */
export type RangeSelection = { start: number; end: number; x: number; y: number };

/**
 * Seçilen aralık için menü: içindeki kayıtları kategoriye ata, elle kayıt ekle ya da sil.
 * Bırakılan noktanın yanında açılır; dışına tıklayınca ya da Esc ile kapanır.
 */
export function RangeMenu({
  selection,
  categories,
  onAddEntry,
  onChanged,
  onClose,
}: {
  selection: RangeSelection;
  categories: Tag[];
  onAddEntry: (start: number, end: number) => void;
  onChanged: () => void;
  onClose: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Gelecek kaydedilemez ve silinecek bir şey de yoktur: bitiş şimdiye kırpılır.
  const end = Math.min(selection.end, Date.now());
  const start = Math.min(selection.start, end);
  const iso = (t: number) => new Date(t).toISOString();
  const run = (f: () => Promise<unknown>) =>
    f().then(
      () => {
        onChanged();
        onClose();
      },
      (e) => setError(message(e)),
    );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const width = 264;
  const left = Math.min(selection.x + 12, window.innerWidth - width - 12);
  const top = Math.min(selection.y, window.innerHeight - 220);
  return (
    <>
      <div className="fixed inset-0 z-40" onPointerDown={onClose} />
      <div
        role="dialog"
        aria-label="Seçilen aralık"
        className="fixed z-50 space-y-3 rounded-lg border bg-popover p-3 text-popover-foreground shadow-lg"
        style={{ left, top, width }}
      >
        <div className="flex items-start justify-between gap-2">
          <div>
            <div className="text-sm font-medium tabular">
              {formatTime(new Date(selection.start))} – {formatTime(new Date(selection.end))}
            </div>
            <div className="text-xs text-muted-foreground">
              {formatDuration((selection.end - selection.start) / 1000)}
            </div>
          </div>
          <Button size="icon-sm" variant="ghost" className="-mt-1 -mr-1" onClick={onClose} aria-label="Kapat">
            <X />
          </Button>
        </div>
        {end > start && (
          <CategorySelect
            value={null}
            onChange={(id) => run(() => api.setRangeCategory(iso(start), iso(end), id))}
            categories={categories}
            noneLabel="Kurallara göre"
            placeholder="İçindeki kayıtları kategoriye ata…"
            className="w-full"
            aria-label="Aralığın kategorisi"
          />
        )}
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
          {end > start &&
            (confirm ? (
              <Button size="sm" variant="destructive" onClick={() => run(() => api.deleteRange(iso(start), iso(end)))}>
                Silinsin
              </Button>
            ) : (
              <Button
                size="icon-sm"
                variant="ghost"
                className="text-muted-foreground hover:text-destructive"
                onClick={() => setConfirm(true)}
                aria-label="Aralıktaki kayıtları sil"
                title="Aralıktaki kayıtları sil"
              >
                <Trash2 />
              </Button>
            ))}
        </div>
        {error && <p className="text-xs text-destructive selectable">{error}</p>}
      </div>
    </>
  );
}

/** Blok kartının altı: bloğu kategoriye ata ya da sil. */
export function BlockActions({
  start,
  end,
  categoryId,
  categories,
  onChanged,
}: {
  start: string;
  end: string;
  categoryId: string | null;
  categories: Tag[];
  onChanged: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (f: () => Promise<unknown>) => f().then(onChanged, (e) => setError(message(e)));

  return (
    <div className="space-y-2 border-t pt-3">
      <div className="flex items-center gap-2">
        <CategorySelect
          value={categoryId}
          onChange={(id) => run(() => api.setRangeCategory(start, end, id))}
          categories={categories}
          noneLabel="Kurallara göre"
          className="min-w-0 flex-1"
          aria-label="Bloğun kategorisi"
        />
        {confirm ? (
          <Button size="sm" variant="destructive" onClick={() => run(() => api.deleteRange(start, end))}>
            Silinsin
          </Button>
        ) : (
          <Button
            size="icon-sm"
            variant="ghost"
            className="text-muted-foreground hover:text-destructive"
            onClick={() => setConfirm(true)}
            aria-label="Bloğu sil"
            title="Bloğu sil"
          >
            <Trash2 />
          </Button>
        )}
      </div>
      {confirm && <p className="text-[11px] text-muted-foreground">Bu bloktaki kayıtlar raporlardan kaldırılır.</p>}
      {error && <p className="text-[11px] text-destructive selectable">{error}</p>}
    </div>
  );
}

/** "Kayıt ekle": bilgisayar dışında geçen süreyi (toplantı, okuma) elle ekler. */
/** Takvimdeki boş alana tıklanınca formu o aralıkla açma isteği. */
export type EntryDraft = { date: string; from: string; to: string; seq: number };

export function ManualEntry({
  day,
  categories,
  onChanged,
  draft,
  onClose,
}: {
  /** Varsayılan tarih (YYYY-MM-DD). */
  day: string;
  categories: Tag[];
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
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!draft) return;
    setDate(draft.date);
    setFrom(draft.from);
    setTo(draft.to);
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
      await api.addManualEntry(label, start.toISOString(), end.toISOString(), category);
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
          {error && <p className="text-xs text-destructive selectable">{error}</p>}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" size="sm" onClick={() => setOpen(false)}>
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
