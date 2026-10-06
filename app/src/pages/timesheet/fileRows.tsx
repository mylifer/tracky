import { useEffect, useRef, useState } from "react";
import { Check, Copy, FileSpreadsheet, Loader2, RefreshCw, Sheet, Trash2, TriangleAlert } from "lucide-react";
import {
  api,
  type EntryKind,
  type FileRow,
  type SheetRows,
  type SheetRowView,
  type Timesheet as TimesheetInfo,
} from "../../api";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/select";
import { parseIsoDate } from "../../lib/dates";
import { Popover, PopoverContent, PopoverTrigger } from "../../components/ui/popover";
import { cn } from "../../lib/utils";
import { friendlyError, toast } from "../../lib/feedback";
import { sameFileRow } from "../../lib/timesheet";
import { KINDS, dayFmt, FIX_LINK, ROW_GRID, type Run } from "./shared";

/**
 * Dosyanın satırlarının durumu: okunduysa Kum'un aktardığı kayıtların dosyadaki satır numarası
 * ve dosyada bulunamayan kayıtlar. Aktarılmış satır yalnızca dosyadaki satırı bilinince düzenlenir.
 */
export type PatchFile = (f: (rows: SheetRowView[]) => SheetRowView[]) => void;

export type FileState =
  { kind: "off" | "loading" | "error" } | { kind: "ready"; rowOf: Map<string, number>; missing: Set<string> };

/** Dosyanın satırları bu süreden yeniyse sayfa açılınca yeniden okunmaz. */
export const FILE_FRESH_MS = 2 * 60_000;
const FILE_CACHE_KEY = "kum.timesheet.file";
/** Okunan dosya satırları (çizelge, dosya ve aya göre); uygulama yeniden açılınca da durur. */
let fileCache: Record<string, { at: number; data: SheetRows }> | null = null;

export function readFileCache(of: string) {
  if (!fileCache) {
    try {
      fileCache = JSON.parse(localStorage.getItem(FILE_CACHE_KEY) ?? "{}");
    } catch {
      fileCache = {};
    }
  }
  return fileCache?.[of] ?? null;
}

/** Ayların satırları tek listede. */
export function mergeFileParts(parts: SheetRows[]): SheetRows {
  return {
    rows: parts.flatMap((p) => p.rows),
    missing: [...new Set(parts.flatMap((p) => p.missing))],
    synced: 0,
  };
}

/** Önbelleğe yazar; en yeni 12 ay tutulur. `at` verilirse okunma anı korunur (yerel düzeltme). */
export function writeFileCache(of: string, data: SheetRows, at = Date.now()) {
  readFileCache(of);
  // Ekrandaki kimlikler saklanmaz: uygulama yeniden açılınca yenileri verilir.
  const rows = data.rows.map(({ uid: _, ...r }) => r);
  const all = { ...fileCache, [of]: { at, data: { ...data, rows } } };
  const keep = Object.entries(all)
    .sort((a, b) => b[1].at - a[1].at)
    .slice(0, 12);
  fileCache = Object.fromEntries(keep);
  try {
    localStorage.setItem(FILE_CACHE_KEY, JSON.stringify(fileCache));
  } catch {
    // Saklanamasa da bu oturumda bellekte durur.
  }
}

/** Dosyanın adı, ekleriyle: "tablodan", "Excel dosyasından"… */
export function fileWords(sheets: boolean) {
  return sheets
    ? { from: "tablodan", to: "tabloya", of: "tablonun", Of: "Tablonun", in: "tabloda", inAdj: "tablodaki" }
    : {
        from: "Excel dosyasından",
        to: "Excel dosyasına",
        of: "Excel dosyasının",
        Of: "Excel dosyasının",
        in: "Excel dosyasında",
        inAdj: "Excel dosyasındaki",
      };
}

/** Betik eski: yeni işlemleri (satırları okuma, değiştirme) tanımıyor. */
const OUTDATED = "betik eski";

/**
 * Dosyadaki satırlar okunamadıysa neden (satır sayısı ve okuma durumu sağ panelde).
 * Betik eskiyse yeni betik kopyalanıp dağıtım güncellenir (adres değişmez).
 */
export function FileError({
  sheets,
  loading,
  error,
  onReload,
}: {
  sheets: boolean;
  loading: boolean;
  error: string;
  onReload: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const where = sheets ? "Tablodaki" : "Excel dosyasındaki";
  const outdated = error.includes(OUTDATED);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(await api.sheetScript());
      setCopied(true);
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  };
  return (
    <div className="space-y-1.5 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <TriangleAlert className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
        <span className="min-w-0 flex-1 selectable">
          {outdated
            ? "Tablodaki betik eski: satırları buradan okuyup değiştirmek için yeni betik gerekiyor."
            : `${where} satırlar okunamadı: ${error}`}
        </span>
        <button
          className={cn(FIX_LINK, "flex items-center gap-1")}
          disabled={loading}
          onClick={onReload}
          title={`${where} satırları yeniden oku`}
        >
          {loading ? <Loader2 className="size-3 animate-spin" /> : <RefreshCw className="size-3" />}
          {loading ? "okunuyor…" : "yenile"}
        </button>
      </div>
      {outdated && (
        <div className="flex flex-wrap items-center gap-2 pl-5.5 text-muted-foreground">
          <Button size="sm" variant="outline" className="h-6 text-[11px]" onClick={copy}>
            {copied ? <Check /> : <Copy />} {copied ? "Kopyalandı" : "Yeni betiği kopyala"}
          </Button>
          <span>
            Tabloda <b>Uzantılar → Apps Script</b>'te içindekini silip yapıştır, kaydet; sonra{" "}
            <b>Dağıt → Dağıtımları yönet → ✎ → Sürüm: Yeni sürüm → Dağıt</b>. Adres değişmez.
          </span>
        </div>
      )}
    </div>
  );
}

/** Dosya satırında düzenlenen alanlar. */
type FileDraft = Pick<FileRow, "start" | "hours" | "kind" | "details" | "party" | "division">;

/**
 * Dosyada Kum dışında girilmiş satır: Kum'da kaydı yok, doğrudan dosyadaki satır değişir.
 * Simgesinden tarihi değiştirilir (satır dosyada yeni gününe taşınır). Silme bildirimden geri
 * alınır; başlangıcı, saati ya da türü eksik satır geri eklenemeyeceği için iki tıklamayla silinir.
 */
export function FileRowItem({
  row,
  sheet,
  divisions,
  runFile,
  patchFile,
}: {
  row: SheetRowView;
  sheet: TimesheetInfo;
  divisions: string[];
  runFile: Run;
  patchFile: PatchFile;
}) {
  const pick = (r: FileRow): FileDraft => ({
    start: r.start,
    hours: r.hours,
    kind: r.kind,
    details: r.details,
    party: r.party,
    division: r.division,
  });
  const [draft, setDraft] = useState<FileDraft>(() => pick(row));
  // Dosyadaki satırın, sıraya giren bütün yazmalardan sonraki hali: her yazma bir öncekinin
  // sonucunu bekler (art arda düzenlemeler "satır değişmiş" diye reddedilmesin).
  const latest = useRef<SheetRowView>(row);
  const pending = useRef(0);
  const rowJson = JSON.stringify(row);
  useEffect(() => {
    if (pending.current > 0) return;
    const r: SheetRowView = JSON.parse(rowJson);
    latest.current = r;
    setDraft(pick(r));
  }, [rowJson]);
  const [confirmDelete, setConfirmDelete] = useState(false);
  useEffect(() => {
    if (!confirmDelete) return;
    const t = setTimeout(() => setConfirmDelete(false), 4000);
    return () => clearTimeout(t);
  }, [confirmDelete]);
  const w = fileWords(!!sheet.sheetUrl);
  const clean = (next: FileDraft) => ({ ...next, details: next.details.trim(), party: next.party.trim() });
  /**
   * Satırı `next` haline getirir (arka planda, sırayla); yazılınca ekrandaki dosya satırı ve
   * numarası güncellenir. Taslak bu sırada ekranda kalır.
   */
  const write = (next: SheetRowView, after?: (written: SheetRowView) => void) => {
    const expect = latest.current;
    latest.current = next;
    pending.current++;
    runFile(async () => {
      try {
        const written = { ...next, row: await api.saveSheetRow(sheet.id, expect, next) };
        if (latest.current === next) latest.current = written;
        patchFile((rows) => rows.map((r) => (sameFileRow(r, expect) ? written : r)));
        after?.(written);
      } catch (e) {
        if (latest.current === next) latest.current = expect;
        throw e;
      } finally {
        pending.current--;
      }
    })();
  };
  const save = (next: FileDraft) => {
    const c = clean(next);
    const cur = latest.current;
    const changed = (Object.keys(c) as (keyof FileDraft)[]).some((k) => c[k] !== cur[k]);
    if (!changed || !c.start || !c.hours) return;
    write({ ...cur, ...c });
  };
  // Dosyaya yeniden yazılabilir: başlangıcı, saati ve türü geçerli (geri ekleme, taşıma).
  const complete = (r: FileDraft) => !!r.start && !!r.hours && KINDS.includes(r.kind as EntryKind);
  const restorable = complete(row);
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveTo, setMoveTo] = useState(row.date);
  /**
   * Satırı başka güne taşır; bildirimden geri alınır (eski gününe döner). Satır başka güne geçince
   * bu bileşen kalkar: geri alma ona bağlı değildir.
   */
  const move = () => {
    const date = moveTo;
    if (!date || date === row.date) return;
    setMoveOpen(false);
    const before = latest.current;
    write({ ...before, ...clean(draft), date }, (moved) =>
      toast(`Satır ${dayFmt.format(parseIsoDate(date))} gününe taşındı`, {
        tone: "success",
        action: {
          label: "Geri al",
          run: () =>
            runFile(async () => {
              const back = { ...before, row: await api.saveSheetRow(sheet.id, moved, before) };
              patchFile((rows) => rows.map((r) => (sameFileRow(r, moved) ? back : r)));
            })(),
        },
      }),
    );
  };
  const remove = () => {
    if (!restorable && !confirmDelete) return setConfirmDelete(true);
    setConfirmDelete(false);
    const gone = latest.current;
    runFile(async () => {
      await api.deleteSheetRow(sheet.id, gone);
      patchFile((rows) => rows.filter((r) => !sameFileRow(r, gone)));
      toast(
        `Satır ${w.from} silindi`,
        restorable
          ? {
              action: {
                label: "Geri al",
                run: () =>
                  runFile(async () => {
                    const at = await api.restoreSheetRow(sheet.id, gone);
                    patchFile((rows) => [...rows, { ...gone, row: at }]);
                    toast(`Satır ${w.to} geri eklendi`, { tone: "success" });
                  })(),
              },
            }
          : { tone: "success" },
      );
    })();
  };
  const kinds = KINDS.includes(draft.kind as EntryKind) || !draft.kind ? KINDS : [...KINDS, draft.kind];
  const options =
    divisions.some((d) => d === draft.division) || !draft.division ? divisions : [...divisions, draft.division];
  const cell = "h-7 px-1.5 text-xs";
  const title = `${w.Of} ${row.row}. satırı; Kum dışında girilmiş. Değişiklikler doğrudan oraya yazılır. Tıkla: tarihi değiştir.`;
  const Icon = sheet.sheetUrl ? Sheet : FileSpreadsheet;

  return (
    <li className="bg-muted/30">
      <div className={cn("grid items-center gap-2 px-4 py-1", ROW_GRID)}>
        <Popover
          open={moveOpen}
          onOpenChange={(o) => {
            setMoveOpen(o);
            if (o) setMoveTo(row.date);
          }}
        >
          <PopoverTrigger asChild>
            <button
              className="-m-1 grid size-6 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
              title={title}
              aria-label={`${w.Of} ${row.row}. satırı: tarihi değiştir`}
            >
              <Icon className="size-3.5" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-64 space-y-2.5 p-3">
            <p className="text-xs text-muted-foreground">
              {w.Of} {row.row}. satırı · Kum dışında girilmiş
            </p>
            <label className="block space-y-1 text-xs font-medium">
              <span>Tarih</span>
              <Input
                type="date"
                className="h-8 text-xs"
                value={moveTo}
                onChange={(e) => setMoveTo(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && move()}
              />
            </label>
            {!complete(draft) && (
              <p className="text-[11px] text-amber-700 dark:text-amber-400">
                Taşımak için başlangıç, saat ve tür dolu olmalı.
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="ghost" onClick={() => setMoveOpen(false)}>
                Vazgeç
              </Button>
              <Button size="sm" disabled={!moveTo || moveTo === row.date || !complete(draft)} onClick={move}>
                Taşı
              </Button>
            </div>
          </PopoverContent>
        </Popover>
        <Input
          type="time"
          className={cell}
          value={draft.start?.slice(0, 5) ?? ""}
          onChange={(e) => setDraft({ ...draft, start: e.target.value ? `${e.target.value}:00` : null })}
          onBlur={() => save(draft)}
          aria-label="Başlangıç"
        />
        <Input
          type="number"
          step="0.25"
          min="0.25"
          className={cn(cell, "w-16 tabular")}
          value={draft.hours == null ? "" : Number(draft.hours.toFixed(2))}
          onChange={(e) => setDraft({ ...draft, hours: e.target.value === "" ? null : Number(e.target.value) })}
          onBlur={() => save(draft)}
          aria-label="Saat"
        />
        <Select value={draft.kind} onValueChange={(kind) => save({ ...draft, kind })}>
          <SelectTrigger size="sm" className="h-7 text-xs" aria-label="Tür">
            <SelectValue placeholder="Tür" />
          </SelectTrigger>
          <SelectContent>
            {kinds.map((k) => (
              <SelectItem key={k} value={k}>
                {k}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          className={cell}
          list="timesheet-details"
          value={draft.details}
          onChange={(e) => setDraft({ ...draft, details: e.target.value })}
          onBlur={() => save(draft)}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Açıklama"
        />
        <Input
          className={cell}
          list="timesheet-parties"
          value={draft.party}
          onChange={(e) => setDraft({ ...draft, party: e.target.value })}
          onBlur={() => save(draft)}
          aria-label="Taraf"
        />
        <Select value={draft.division} onValueChange={(division) => save({ ...draft, division })}>
          <SelectTrigger size="sm" className="h-7 min-w-0 text-xs" aria-label="Birim">
            <SelectValue placeholder="Birim seç" />
          </SelectTrigger>
          <SelectContent>
            {options.map((d) => (
              <SelectItem key={d} value={d}>
                {d}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          size="icon-sm"
          variant={confirmDelete ? "destructive" : "ghost"}
          className={cn("size-7", !confirmDelete && "text-muted-foreground hover:text-destructive")}
          aria-label={confirmDelete ? `Satırı ${w.from} sil: onayla` : `Satırı ${w.from} sil`}
          title={
            restorable
              ? `Sil: satır ${w.from} silinir; bildirimden geri alınır`
              : confirmDelete
                ? `Onaylamak için tekrar tıkla: satır ${w.from} silinir`
                : `Sil: başlangıcı, saati ya da türü eksik satır geri eklenemez; iki tıklamayla silinir`
          }
          onClick={remove}
        >
          {confirmDelete ? <Check /> : <Trash2 />}
        </Button>
      </div>
    </li>
  );
}
