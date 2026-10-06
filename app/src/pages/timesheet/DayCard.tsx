import { useMemo, useState } from "react";
import { CalendarDays, Copy, Loader2, RotateCcw, Sparkles, WandSparkles } from "lucide-react";
import {
  api,
  formatDuration,
  type EntryView,
  type SheetRowView,
  type Tag,
  type Timesheet as TimesheetInfo,
  type TimesheetConfig,
  type TimesheetDay,
  type UnassignedMeeting,
} from "../../api";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { isoDate, parseIsoDate, today } from "../../lib/dates";
import { ProjectSelect } from "../../components/ProjectSelect";
import { cn } from "../../lib/utils";
import { toast } from "../../lib/feedback";
import { UNASSIGNED_MIN, hoursDiff, needsDetails } from "../../lib/timesheet";
import {
  dayFmt,
  num,
  timeFmt,
  WARN_BADGE,
  FIX_LINK,
  ROW_GRID,
  IGNORE,
  actual,
  worked,
  manDays,
  signedHours,
  projectName,
  defaultDivision,
  type Run,
  useBusy,
} from "./shared";
import { copyPreviousDetails, aiWriteDays } from "./actions";
import { type PatchFile, type FileState, FileRowItem } from "./fileRows";
import { EntryRow, SelectBox } from "./EntryRow";

export function DayCard({
  day,
  config,
  sheet,
  projects,
  onOpenDay,
  onReviewDay,
  run,
  runFile,
  patchFile,
  reloadFile,
  file,
  outside,
  ai,
  selected,
  onToggle,
  alwaysShow,
}: {
  day: TimesheetDay;
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  onOpenDay: (iso: string) => void;
  onReviewDay: (iso: string) => void;
  run: Run;
  /** Dosyaya da yazan işlemler (arka planda; beklenmez). */
  runFile: Run;
  /** Yazılan değişikliği ekrandaki dosya satırlarına uygular. */
  patchFile: PatchFile;
  /** Dosyayı yeniden okur (Kum'un satırı yeniden gönderilince eşlensin). */
  reloadFile: () => void;
  file: FileState;
  /** Günün dosyada Kum dışında girilmiş satırları. */
  outside: SheetRowView[];
  /** Yapay zekâyla yazma açık. */
  ai: boolean;
  selected: Set<string>;
  onToggle: (keys: string[], on: boolean) => void;
  alwaysShow: boolean;
}) {
  const [confirmReset, setConfirmReset] = useState(false);
  // Yapay zekâ yazıyor (düğme kilitli, dönen simge).
  const [writing, setWriting] = useState(false);
  // Satır ekleme ve önceki günden kopyalama sürüyor.
  const [adding, guardAdd] = useBusy();
  const [copying, guardCopy] = useBusy();
  const aiWrite = async (rewrite: boolean) => {
    setWriting(true);
    try {
      await run(() => aiWriteDays(sheet.id, [day.date], rewrite))();
    } finally {
      setWriting(false);
    }
  };
  // Satırın birim seçeneği: çizelgenin projelerinin birimleri ve dosyadaki birimler.
  const divisions = useMemo(() => {
    const list: string[] = [];
    const add = (d: string) => {
      const t = d.trim();
      if (t && !list.some((x) => x.toLocaleLowerCase("tr") === t.toLocaleLowerCase("tr"))) list.push(t);
    };
    for (const m of sheet.projects) add(m.division || projectName(projects, m.projectId) || "");
    for (const d of sheet.divisions) add(d);
    return list;
  }, [sheet, projects]);
  const date = parseIsoDate(day.date);
  const outsideHours = outside.reduce((s, r) => s + (r.hours ?? 0), 0);
  const total = day.entries.reduce((s, e) => s + e.hours, 0) + outsideHours;
  const totalActual = day.entries.reduce((s, e) => s + worked(e), 0);
  const exported = day.entries.length > 0 && day.entries.every((e) => e.exported);
  const empty =
    day.entries.length === 0 &&
    outside.length === 0 &&
    day.unassignedSeconds < 60 &&
    day.meetings.length === 0 &&
    day.hidden === 0;
  const weekend = date.getDay() === 0 || date.getDay() === 6;
  if (empty && weekend && !alwaysShow) return null;
  // Günlük saatten sapma (bugün ve öncesi; ileri tarihli gün henüz bitmedi).
  const diff = day.date <= isoDate(today()) ? hoursDiff(day, config.dayHours, outsideHours) : 0;
  // Kum'un satırları ve dosyada Kum dışında girilmiş satırlar, başlangıca göre (başlangıcı
  // olmayan dosya satırları sonda).
  const lines: ({ entry: EntryView; row?: undefined } | { row: SheetRowView; entry?: undefined })[] = [
    ...day.entries.map((entry) => ({ entry })),
    ...outside.map((row) => ({ row })),
  ];
  const startOf = (l: (typeof lines)[number]) => (l.entry ? l.entry.start : (l.row.start ?? "99"));
  lines.sort((a, b) => startOf(a).localeCompare(startOf(b)));
  const missing = day.entries.some(needsDetails);
  const open = day.entries.filter((e) => !e.exported);
  // Kaydedilmiş (düzenlenmiş, elle eklenmiş) ya da gizlenmiş satır varsa sıfırlanabilir.
  const touched = open.some((e) => e.id) || day.hidden > 0;
  const openKeys = open.map((e) => e.key);
  const chosen = openKeys.filter((k) => selected.has(k)).length;
  const sheetProjects = sheet.projects
    .map((m) => projects.find((p) => p.id === m.projectId))
    .filter((p): p is Tag => !!p);

  const addRow = (projectId: string) =>
    guardAdd(
      run(() =>
        api.saveTimesheetEntry(null, {
          date: day.date,
          start: "09:00:00",
          hours: 1,
          actualHours: null,
          kind: "Working",
          details: "",
          party: sheet.projects.find((m) => m.projectId === projectId)?.party || sheet.defaultParty,
          projectId,
          division: defaultDivision(sheet, projects, projectId),
          coverage: [],
        }),
      ),
    )();

  return (
    <section id={`gun-${day.date}`} className="scroll-mt-4 rounded-xl border bg-card shadow-xs">
      <div className="flex flex-wrap items-center gap-2 px-4 py-2.5">
        <span className="text-[13px] font-semibold capitalize">{dayFmt.format(date)}</span>
        <span className="text-xs text-muted-foreground tabular">
          {total ? manDays(total, config.dayHours) : "—"}
          {total > 0 && <span title="Takip edilen gerçek süre"> · gerçek {actual(totalActual)}</span>}
        </span>
        {exported && (
          <Badge variant="outline" className="border-success/40 text-success">
            Gönderildi
          </Badge>
        )}
        {diff !== 0 && (
          <Badge
            variant="outline"
            className={WARN_BADGE}
            title={`Günlük ${num.format(config.dayHours)} saatten ${diff < 0 ? "az" : "fazla"}`}
          >
            {signedHours(diff)}
          </Badge>
        )}
        {day.unassignedSeconds >= UNASSIGNED_MIN && (
          <span className="flex items-center gap-1 text-xs text-muted-foreground">
            <button
              className="rounded underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onReviewDay(day.date)}
              title="Gözden geçir: atanmamış süreyi site ve uygulamaya göre projeye ata"
            >
              Atanmamış {formatDuration(day.unassignedSeconds)} · gözden geçir
            </button>
            <span aria-hidden>·</span>
            <button
              className="rounded underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => onOpenDay(day.date)}
              title="Takvimde aç: blokları ya da aralıkları projeye ata"
            >
              takvim
            </button>
          </span>
        )}
        {day.hidden > 0 && (
          <button
            className="rounded text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            title="Silinen satırları geri getir"
            onClick={run(async () => {
              const ids = await api.restoreHiddenEntries(sheet.id, day.date);
              toast(`${ids.length} satır geri getirildi`, { tone: "success" });
            })}
          >
            {day.hidden} silinmiş · geri getir
          </button>
        )}
        <span className="ml-auto flex items-center gap-1.5">
          {missing && (
            <Button
              size="sm"
              variant="ghost"
              disabled={copying}
              title="Önceki günlerin aynı projedeki açıklamalarını boş satırlara yaz"
              onClick={guardCopy(run(() => copyPreviousDetails(sheet.id, day.date)))}
            >
              <Copy /> Önceki günden kopyala
            </Button>
          )}
          {ai && open.length > 0 && (
            <Button
              size="sm"
              variant="ghost"
              disabled={writing}
              title="Boş ya da otomatik gelen açıklamaları Claude yazar, elle yazdıklarına dokunulmaz. Bildirimden geri alınır."
              onClick={() => aiWrite(false)}
            >
              {writing ? <Loader2 className="animate-spin" /> : <Sparkles />} Yapay zekâyla yaz
            </Button>
          )}
          {ai && open.some((e) => e.details.trim()) && (
            <Button
              size="icon-sm"
              variant="ghost"
              disabled={writing}
              aria-label="Hepsini yapay zekâyla yeniden yaz"
              title="Hepsini yeniden yaz: elle yazılanlar dahil aktarılmamış bütün açıklamaları Claude yazar. Bildirimden geri alınır."
              onClick={() => aiWrite(true)}
            >
              <WandSparkles />
            </Button>
          )}
          {touched &&
            (confirmReset ? (
              <Button
                size="sm"
                variant="destructive"
                onClick={() => {
                  // Onay tek kullanımlık: sonraki tıklama yeniden onay istesin.
                  setConfirmReset(false);
                  run(() => api.resetTimesheetDay(sheet.id, day.date))();
                }}
              >
                Düzenlemeler silinsin, yeniden öner
              </Button>
            ) : (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setConfirmReset(true)}
                title="Düzenlemeleri, birleştirmeleri, elle eklenen ve silinen satırları geri al; satırlar takipten yeniden önerilir. Gönderilenlere dokunulmaz."
              >
                <RotateCcw /> Sıfırla
              </Button>
            ))}
          {sheetProjects.length === 1 && (
            <Button size="sm" variant="outline" disabled={adding} onClick={() => addRow(sheetProjects[0].id)}>
              + Satır
            </Button>
          )}
          {sheetProjects.length > 1 && (
            <ProjectSelect
              value=""
              projects={sheetProjects}
              placeholder="+ Satır…"
              className="w-32"
              aria-label="Satır ekle: proje"
              onChange={addRow}
            />
          )}
        </span>
      </div>
      {lines.length > 0 && (
        // Dar pencerede açıklama sütunu ezilmesin: satırlar kart içinde yatay kayar.
        <div className="overflow-x-auto border-t">
          <div className={cn("grid items-center gap-2 px-4 pt-2 text-[11px] text-muted-foreground", ROW_GRID)}>
            {openKeys.length > 0 ? (
              <SelectBox
                checked={chosen === openKeys.length}
                indeterminate={chosen > 0 && chosen < openKeys.length}
                onChange={(on) => onToggle(openKeys, on)}
                label={`${dayFmt.format(date)}: gönderilmemiş satırların hepsini seç`}
              />
            ) : (
              <span />
            )}
            <span>Başlangıç</span>
            <span>Saat</span>
            <span>Tür</span>
            <span>Açıklama</span>
            <span>Taraf</span>
            <span>Birim</span>
            <span />
          </div>
          <ul className="pb-1.5">
            {lines.map(({ entry: e, row }) =>
              e ? (
                <EntryRow
                  key={e.key}
                  entry={e}
                  divisions={divisions}
                  projects={projects}
                  run={run}
                  runFile={runFile}
                  patchFile={patchFile}
                  reloadFile={reloadFile}
                  file={file}
                  sheets={!!sheet.sheetUrl}
                  sheetId={sheet.id}
                  selected={selected.has(e.key)}
                  onSelect={(on) => onToggle([e.key], on)}
                />
              ) : (
                <FileRowItem
                  key={`dosya-${row.uid}`}
                  row={row}
                  sheet={sheet}
                  divisions={divisions}
                  runFile={runFile}
                  patchFile={patchFile}
                />
              ),
            )}
          </ul>
        </div>
      )}
      {day.meetings.length > 0 && <MeetingList meetings={day.meetings} projects={projects} run={run} />}
    </section>
  );
}

/**
 * Takvimde olup hiçbir projeye düşmeyen toplantılar. Seçilen proje serinin tüm tekrarlarına
 * uygulanır (haftalık toplantı bir kez atanır); "Yoksay" seriyi zaman çizelgesinden çıkarır.
 */
function MeetingList(props: { meetings: UnassignedMeeting[]; projects: Tag[]; run: Run }) {
  return (
    <div className="border-t px-4 py-2">
      <div className="flex items-center gap-1.5 pb-1 text-[11px] text-muted-foreground">
        <CalendarDays className="size-3.5" /> Takvimden, projesi belli olmayan toplantılar
      </div>
      <MeetingRows {...props} />
    </div>
  );
}

/**
 * Toplantılar ve proje seçimi (gün kartında ve dönem denetiminde). Emin olunan öneri seçicinin
 * yanında "→ Proje" olarak durur (gerekçesi ipucunda); tıklayınca seri o projeye atanır.
 * Birden çok seri için öneri varsa hepsi tek düğmeyle atanır.
 */
export function MeetingRows({ meetings, projects, run }: { meetings: UnassignedMeeting[]; projects: Tag[]; run: Run }) {
  const nameOf = (id: string) => projectName(projects, id);
  // Seri başına bir öneri (aynı gün iki tekrar olsa da bir kez atanır); adı bilinmeyen
  // (seçicide olmayan) proje önerilmez.
  const suggested = new Map<string, string>();
  for (const m of meetings) {
    if (m.suggestion && nameOf(m.suggestion.projectId)) suggested.set(m.uid, m.suggestion.projectId);
  }
  const assignAll = run(async () => {
    for (const [uid, projectId] of suggested) await api.assignMeeting(uid, projectId);
  });
  return (
    <ul className="space-y-1">
      {meetings.map((m) => {
        const s = m.suggestion;
        const name = s ? nameOf(s.projectId) : undefined;
        return (
          <li key={`${m.uid}-${m.start}`} className="flex flex-wrap items-center gap-2 text-xs">
            <span className="w-24 shrink-0 text-muted-foreground tabular">
              {timeFmt.format(new Date(m.start))}–{timeFmt.format(new Date(m.end))}
            </span>
            <span className="min-w-0 flex-1 truncate" title={m.location || undefined}>
              {m.subject || "(konusuz)"}
              <span className="ml-1.5 text-muted-foreground">{m.online ? "Online" : "F2F"}</span>
            </span>
            {s && name && (
              <button
                type="button"
                className="max-w-40 truncate rounded-md border border-dashed px-1.5 py-0.5 text-[11px] text-muted-foreground hover:border-solid hover:bg-accent hover:text-foreground"
                title={`Öneri: ${s.reason}`}
                aria-label={`${m.subject} toplantısını ${name} projesine ata (${s.reason})`}
                onClick={run(() => api.assignMeeting(m.uid, s.projectId))}
              >
                → {name}
              </button>
            )}
            <ProjectSelect
              value=""
              projects={projects}
              placeholder="Projeye ata…"
              extra={[{ value: IGNORE, label: "Zaman çizelgesine alma" }]}
              className="w-44"
              aria-label={`${m.subject} projesi`}
              onChange={(v) => run(() => api.assignMeeting(m.uid, v === IGNORE ? null : v))()}
            />
          </li>
        );
      })}
      {suggested.size > 1 && (
        <li className="flex justify-end">
          <button className={cn(FIX_LINK, "text-[11px]")} onClick={assignAll}>
            Önerilenleri ata ({suggested.size})
          </button>
        </li>
      )}
    </ul>
  );
}
