import { useEffect, useRef, useState } from "react";
import { Check, Trash2, TriangleAlert } from "lucide-react";
import { api, type EntryKind, type EntryView, type Tag } from "../../api";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/select";
import { tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { toast } from "../../lib/feedback";
import { isStale } from "../../lib/timesheet";
import { KINDS, ROW_GRID, EDITABLE, actual, type Run } from "./shared";
import { dismissRows, refreshRows } from "./actions";
import { type PatchFile, type FileState, fileWords } from "./fileRows";

export function EntryRow({
  entry,
  divisions,
  projects,
  run,
  runFile,
  patchFile,
  reloadFile,
  file,
  sheets,
  sheetId,
  selected,
  onSelect,
}: {
  entry: EntryView;
  /** Çizelgenin birimleri (satırın birimi bunlardan seçilir). */
  divisions: string[];
  projects: Tag[];
  run: Run;
  runFile: Run;
  patchFile: PatchFile;
  reloadFile: () => void;
  file: FileState;
  /** Çizelge Google Sheets'e bağlı (değilse Excel). */
  sheets: boolean;
  sheetId: string;
  selected: boolean;
  onSelect: (on: boolean) => void;
}) {
  // Aktarılmış satır dosyadaki satırı bulunduysa düzenlenir; değişiklik dosyaya da yazılır.
  const fileRow = entry.exported && entry.id && file.kind === "ready" ? file.rowOf.get(entry.id) : undefined;
  const lost = entry.exported && !!entry.id && file.kind === "ready" && file.missing.has(entry.id);
  const editable = !entry.exported || fileRow !== undefined;
  const w = fileWords(sheets);
  const [draft, setDraft] = useState(entry);
  // Her yeniden yüklemede satırlar yeni nesne olarak gelir; yalnızca içerik değişince taslak
  // yenilenir ve henüz kaydedilmemiş yazılanlar yeni içeriğin üzerinde kalır (başka satırın
  // kaydı ya da takvim yenilemesi yazılanı silmesin).
  const base = useRef(entry);
  const entryJson = JSON.stringify(entry);
  useEffect(() => {
    const prev = base.current;
    const next: EntryView = JSON.parse(entryJson);
    base.current = next;
    setDraft((d) => {
      const out = { ...next };
      for (const k of EDITABLE) if (d[k] !== prev[k]) Object.assign(out, { [k]: d[k] });
      return out;
    });
  }, [entryJson]);
  // Canlı satır ilk düzenlemede kaydedilir (aralıklarıyla); sonra kimliğiyle güncellenir.
  // Aktarılmış satır önce dosyadaki satırına, sonra Kum'a yazılır.
  const save = (next: EntryView) =>
    entry.exported
      ? runFile(() => api.saveTimesheetEntry(entry.id, next, fileRow))()
      : run(() => api.saveTimesheetEntry(entry.id, next))();
  const commit = () => {
    const next = { ...draft, details: draft.details.trim(), party: draft.party.trim() };
    // Dosyadaki satırın açıklaması boşaltılmaz.
    if (entry.exported && !next.details) return setDraft({ ...draft, details: entry.details });
    // Boş ya da sıfır/eksi süre kaydedilmez (dosyaya da 0 yazılırdı): eski süre geri gelir.
    if (!(next.hours > 0)) return setDraft({ ...next, hours: entry.hours });
    if (EDITABLE.some((k) => next[k] !== entry[k])) save(next);
  };
  // Aktarılmış satır dosyadan da silinir; geri alınınca satır geri gelir ve yeniden gönderilir.
  const remove = () => {
    if (!entry.exported || !entry.id) return run(() => dismissRows([entry]))();
    const id = entry.id;
    return runFile(async () => {
      await api.dismissTimesheetEntry(id, entry, fileRow);
      patchFile((rows) => rows.filter((r) => r.entryId !== id));
      toast(`Satır ${w.from} da silindi`, {
        action: {
          label: "Geri al",
          run: () =>
            runFile(async () => {
              await api.undismissTimesheetEntries([id]);
              await api.exportTimesheet(sheetId, [{ id, entry }]);
              toast(`Satır ${w.to} geri eklendi`, { tone: "success" });
              reloadFile();
            })(),
        },
      });
    })();
  };
  const project = projects.find((p) => p.id === entry.projectId);
  const options =
    divisions.some((d) => d === draft.division) || !draft.division ? divisions : [...divisions, draft.division];
  const cell = "h-7 px-1.5 text-xs";
  // Açıklaması boş satır gönderilemez: hafifçe vurgulanır.
  const missing = !entry.exported && !draft.details.trim();
  const stale = isStale(entry);
  const sentTitle =
    fileRow !== undefined
      ? `Gönderildi: ${w.of} ${fileRow}. satırı. Değişiklikler oraya da yazılır.`
      : lost
        ? `Gönderildi, ama ${w.in} bulunamadı (orada silinmiş ya da başlangıcı değiştirilmiş olabilir).`
        : file.kind === "loading"
          ? `Gönderildi; ${w.inAdj} satırı aranıyor…`
          : "Gönderildi";

  return (
    <li className={cn(stale && "bg-amber-500/5")}>
      <div className={cn("grid items-center gap-2 px-4 py-1", ROW_GRID, !editable && "text-muted-foreground")}>
        {!entry.exported ? (
          <SelectBox checked={selected} onChange={onSelect} label="Satırı seç" />
        ) : lost ? (
          <TriangleAlert className="size-3.5 text-amber-600 dark:text-amber-400" aria-label={sentTitle}>
            <title>{sentTitle}</title>
          </TriangleAlert>
        ) : (
          <Check className="size-3.5 text-success" aria-label={sentTitle}>
            <title>{sentTitle}</title>
          </Check>
        )}
        <Input
          type="time"
          className={cell}
          disabled={!editable}
          value={draft.start.slice(0, 5)}
          onChange={(e) => setDraft({ ...draft, start: `${e.target.value}:00` })}
          onBlur={commit}
          aria-label="Başlangıç"
        />
        <div className="flex items-center gap-1.5">
          <Input
            type="number"
            step="0.25"
            min="0.25"
            className={cn(cell, "w-16 tabular")}
            disabled={!editable}
            value={Number(draft.hours.toFixed(2))}
            onChange={(e) => setDraft({ ...draft, hours: Number(e.target.value) })}
            onBlur={commit}
            aria-label="Saat"
          />
          {draft.actualHours != null && (
            <span
              className="truncate text-[11px] text-muted-foreground tabular"
              title="Takip edilen gerçek süre; saat çeyreğe yuvarlanır"
            >
              {actual(draft.actualHours)}
            </span>
          )}
        </div>
        <Select value={draft.kind} disabled={!editable} onValueChange={(v) => save({ ...draft, kind: v as EntryKind })}>
          <SelectTrigger size="sm" className="h-7 text-xs" aria-label="Tür">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {KINDS.map((k) => (
              <SelectItem key={k} value={k}>
                {k}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          className={cn(cell, missing && "border-amber-500/50 bg-amber-500/5")}
          data-empty={missing || undefined}
          title={missing ? "Açıklama boş: bu satır gönderilemez" : undefined}
          disabled={!editable}
          list="timesheet-details"
          value={draft.details}
          placeholder={editable ? (draft.kind === "Working" ? "Ne yaptın?" : "Toplantı konusu?") : ""}
          onChange={(e) => setDraft({ ...draft, details: e.target.value })}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Açıklama"
        />
        <Input
          className={cell}
          disabled={!editable}
          list="timesheet-parties"
          value={draft.party}
          onChange={(e) => setDraft({ ...draft, party: e.target.value })}
          onBlur={commit}
          aria-label="Taraf"
        />
        <Select value={draft.division} disabled={!editable} onValueChange={(division) => save({ ...draft, division })}>
          <SelectTrigger
            size="sm"
            className="h-7 min-w-0 text-xs"
            aria-label="Birim"
            title={project ? `Proje: ${project.name}` : undefined}
          >
            <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} />
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
        {editable ? (
          <Button
            size="icon-sm"
            variant="ghost"
            className="size-7 text-muted-foreground hover:text-destructive"
            aria-label={entry.exported ? `Satırı ${w.from} da sil` : "Satırı sil"}
            title={
              entry.exported
                ? `Sil: satır ${w.from} da silinir; bildirimden geri alınır`
                : "Sil; bildirimden geri alınır"
            }
            onClick={remove}
          >
            <Trash2 />
          </Button>
        ) : (
          <span />
        )}
      </div>
      {stale && entry.id && (
        <div className="flex flex-wrap items-center gap-1.5 px-4 pb-1.5 pl-[42px] text-[11px] text-amber-700 dark:text-amber-400">
          <TriangleAlert className="size-3" />
          {entry.stale! > 0
            ? `Takipte değişti: işin bir kısmı raporda başka projeye alınmış, projede ${actual(entry.stale!)} kaldı.`
            : "Takipte değişti: bu satırın işi raporda başka projeye alınmış."}
          {entry.exported && ` Güncellenince ${w.inAdj} satırı da değişir.`}
          <button
            className="rounded font-medium underline underline-offset-2 hover:text-foreground disabled:opacity-50"
            disabled={entry.exported && fileRow === undefined}
            onClick={
              entry.exported
                ? runFile(async () => {
                    const id = entry.id!;
                    // İşi kalmayan satır dosyadan da kaldırılır.
                    if ((await api.refreshTimesheetEntries([id])) > 0)
                      patchFile((rows) => rows.filter((r) => r.entryId !== id));
                    toast("Satır güncellendi", { tone: "success" });
                  })
                : run(() => refreshRows([entry.id!]))
            }
          >
            {entry.stale! > 0 ? "Güncelle" : "Kaldır"}
          </button>
        </div>
      )}
    </li>
  );
}

/** Satır seçim kutusu (yarı seçili hali de olur). */
export function SelectBox({
  checked,
  indeterminate = false,
  onChange,
  label,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (on: boolean) => void;
  label: string;
}) {
  return (
    <input
      type="checkbox"
      className="size-3.5 cursor-pointer accent-primary"
      checked={checked}
      ref={(el) => {
        if (el) el.indeterminate = indeterminate;
      }}
      onChange={(e) => onChange(e.target.checked)}
      aria-label={label}
    />
  );
}
