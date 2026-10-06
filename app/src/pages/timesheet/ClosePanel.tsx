import { useState } from "react";
import { CircleCheck, ClipboardCheck, FileSpreadsheet, Loader2, Sheet, Sparkles, TriangleAlert, X } from "lucide-react";
import {
  formatDuration,
  type EntryView,
  type Tag,
  type Timesheet as TimesheetInfo,
  type TimesheetConfig,
  type TimesheetDay,
} from "../../api";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { isoDate, parseIsoDate, today } from "../../lib/dates";
import { cn } from "../../lib/utils";
import { type CloseReport } from "../../lib/timesheet";
import { dayFmt, num, WARN_BADGE, FIX_LINK, signedHours, type Run, useBusy } from "./shared";
import { copyPreviousDetails, aiWriteDays, refreshRows } from "./actions";
import { MeetingRows } from "./DayCard";

/**
 * Dönemi kapatmadan önce denetim: atanmamış süre, projesi belli olmayan toplantılar, günlük
 * saati tutmayan ya da kaydı olmayan iş günleri, açıklaması boş ve takipte değişen satırlar;
 * her birinin yanında düzeltme bağlantısı. Boş açıklama ve takipte değişen satır gönderimi
 * engeller, diğerleri uyarıdır. Ana düğme dönemin gönderilmemiş satırlarını gönderir.
 */
export function ClosePanel({
  title,
  report,
  days,
  config,
  sheet,
  projects,
  ready,
  ai,
  pending,
  onClose,
  onSend,
  onOpenDay,
  onReviewDay,
  onShowDay,
  run,
}: {
  title: string;
  report: CloseReport;
  days: TimesheetDay[];
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  /** Ekrandaki günler güncel ve gönderim sürmüyor. */
  ready: boolean;
  /** Yapay zekâyla yazma açık. */
  ai: boolean;
  /** Dönemin gönderilmemiş satırları. */
  pending: EntryView[];
  onClose: () => void;
  onSend: () => void;
  onOpenDay: (iso: string) => void;
  onReviewDay: (iso: string) => void;
  /** Günün kartını açıp gösterir; `focusEmpty` ise ilk boş açıklamaya odaklanır. */
  onShowDay: (iso: string, focusEmpty?: boolean) => void;
  run: Run;
}) {
  const label = (iso: string) => <span className="w-24 shrink-0 capitalize">{dayFmt.format(parseIsoDate(iso))}</span>;
  const show = (iso: string, focusEmpty = false) => (
    <button className={FIX_LINK} onClick={() => onShowDay(iso, focusEmpty)}>
      göster
    </button>
  );
  const calendarLink = (iso: string) => (
    <button className={FIX_LINK} onClick={() => onOpenDay(iso)} title="Takvimde aç: blokları ya da aralıkları ata">
      takvim
    </button>
  );
  const sendLabel = sheet.sheetUrl ? "Sheets'e gönder" : "Excel'e aktar";
  const isBlocked = report.blocking > 0;
  const [copying, guardCopy] = useBusy();
  // Yapay zekâyla yazılacak günler: bugüne kadar, aktarılmamış satırı olanlar. Yazarken ilerleme.
  const todayIso = isoDate(today());
  const aiDays = days.filter((d) => d.date <= todayIso && d.entries.some((e) => !e.exported)).map((d) => d.date);
  const [aiDone, setAiDone] = useState<number | null>(null);
  const writeAll = async () => {
    setAiDone(0);
    try {
      await run(() => aiWriteDays(sheet.id, aiDays, false, setAiDone))();
    } finally {
      setAiDone(null);
    }
  };
  const staleIds = report.stale.flatMap((s) => s.rows.map((e) => e.id).filter((id): id is string => !!id));

  return (
    <section className="rounded-xl border bg-card shadow-xs" aria-label={`${title}: denetim`}>
      <div className="flex items-center gap-2 border-b px-4 py-2.5">
        <ClipboardCheck className="size-4 text-muted-foreground" />
        <span className="text-[13px] font-semibold">{title}</span>
        <span className="text-xs text-muted-foreground">
          {report.issues === 0 ? "göndermeden önce denetim" : `${report.issues} konu`}
        </span>
        <button
          aria-label="Kapat"
          className="-m-1 ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={onClose}
        >
          <X className="size-3.5" />
        </button>
      </div>
      {report.issues === 0 ? (
        <div className="m-3 flex items-start gap-2 rounded-lg border border-success/30 bg-success/10 px-3 py-2 text-xs">
          <CircleCheck className="mt-0.5 size-3.5 shrink-0 text-success" />
          <span>
            <b className="text-success">Hazır.</b> Açıklamalar dolu, iş günleri {num.format(config.dayHours)} saat;
            atanmamış süre ya da toplantı yok.
            {pending.length === 0 && " Gönderilecek satır kalmadı."}
          </span>
        </div>
      ) : (
        <ul className="divide-y">
          {report.stale.length > 0 && (
            <CheckGroup
              blocking
              title="Takipte değişen satırlar"
              hint="İşi raporda başka projeye alınmış; güncellenmeden gönderilmez."
            >
              {report.stale.map((s) => (
                <li key={s.date} className="flex flex-wrap items-center gap-2">
                  {label(s.date)}
                  <span>{s.rows.length} satır</span>
                  <span className="ml-auto flex gap-2">
                    <button
                      className={FIX_LINK}
                      onClick={run(() => refreshRows(s.rows.map((e) => e.id).filter((id): id is string => !!id)))}
                    >
                      güncelle
                    </button>
                    {show(s.date)}
                  </span>
                </li>
              ))}
              {report.stale.length > 1 && (
                <li className="flex justify-end">
                  <button className={cn(FIX_LINK, "text-[11px]")} onClick={run(() => refreshRows(staleIds))}>
                    Hepsini güncelle ({staleIds.length})
                  </button>
                </li>
              )}
            </CheckGroup>
          )}
          {report.details.length > 0 && (
            <CheckGroup blocking title="Açıklaması boş satırlar" hint="Doldurulmadan gönderilmez.">
              {report.details.map((d) => (
                <li key={d.date} className="flex flex-wrap items-center gap-2">
                  {label(d.date)}
                  <span>{d.rows} satır</span>
                  <span className="ml-auto flex gap-2">
                    <button
                      className={FIX_LINK}
                      disabled={copying}
                      onClick={guardCopy(run(() => copyPreviousDetails(sheet.id, d.date)))}
                    >
                      önceki günden kopyala
                    </button>
                    {ai && (
                      <button
                        className={FIX_LINK}
                        disabled={aiDone !== null}
                        onClick={run(() => aiWriteDays(sheet.id, [d.date], false))}
                      >
                        yapay zekâyla yaz
                      </button>
                    )}
                    {show(d.date, true)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.unassigned.length > 0 && (
            <CheckGroup title="Atanmamış süre" hint="Projeye atanmayan iş zaman çizelgesine girmez.">
              {report.unassigned.map((u) => (
                <li key={u.date} className="flex flex-wrap items-center gap-2">
                  {label(u.date)}
                  <span className="tabular">{formatDuration(u.seconds)}</span>
                  <span className="ml-auto flex gap-2">
                    <button className={FIX_LINK} onClick={() => onReviewDay(u.date)}>
                      gözden geçir
                    </button>
                    {calendarLink(u.date)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.meetings.length > 0 && (
            <CheckGroup
              title="Projesi belli olmayan toplantılar"
              hint="Seçilen proje serinin tüm tekrarlarına uygulanır."
            >
              {report.meetings.map((iso) => (
                <li key={iso} className="flex items-start gap-2">
                  <span className="pt-1">{label(iso)}</span>
                  <div className="min-w-0 flex-1">
                    <MeetingRows
                      meetings={days.find((d) => d.date === iso)?.meetings ?? []}
                      projects={projects}
                      run={run}
                    />
                  </div>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.hours.length > 0 && (
            <CheckGroup
              title="Saati tutmayan iş günleri"
              hint={`Günlük ${num.format(config.dayHours)} saat bekleniyor.`}
            >
              {report.hours.map((h) => (
                <li key={h.date} className="flex flex-wrap items-center gap-2">
                  {label(h.date)}
                  <span className="tabular">{num.format(h.hours)} sa</span>
                  <Badge variant="outline" className={WARN_BADGE}>
                    {signedHours(h.diff)}
                  </Badge>
                  <span className="ml-auto flex gap-2">
                    {calendarLink(h.date)}
                    {show(h.date)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
          {report.empty.length > 0 && (
            <CheckGroup
              title="Kaydı olmayan iş günleri"
              hint="İzin ya da tatilse geçebilirsin; değilse elle satır ekle."
            >
              {report.empty.map((iso) => (
                <li key={iso} className="flex flex-wrap items-center gap-2">
                  {label(iso)}
                  <span className="ml-auto flex gap-2">
                    {calendarLink(iso)}
                    {show(iso)}
                  </span>
                </li>
              ))}
            </CheckGroup>
          )}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2 border-t px-4 py-2.5">
        <p className="mr-auto text-[11px] text-muted-foreground">
          {isBlocked
            ? "Açıklaması boş ya da takipte değişen satırlar düzeltilmeden gönderilmez."
            : report.issues > 0
              ? "Uyarılar göndermeyi engellemez."
              : ""}
        </p>
        {ai && aiDays.length > 0 && (
          <Button
            size="sm"
            variant="outline"
            disabled={!ready || aiDone !== null}
            title="Boş ya da otomatik gelen açıklamaları Claude yazar (gün başına bir istek); elle yazdıklarına dokunulmaz."
            onClick={writeAll}
          >
            {aiDone !== null ? <Loader2 className="animate-spin" /> : <Sparkles />}
            {aiDone !== null ? `Yazılıyor… ${aiDone}/${aiDays.length} gün` : "Boş açıklamaları yapay zekâyla yaz"}
          </Button>
        )}
        <Button
          size="sm"
          disabled={!ready || isBlocked || pending.length === 0}
          title={
            isBlocked
              ? "Önce açıklaması boş ve takipte değişen satırları düzelt"
              : pending.length === 0
                ? "Gönderilecek satır yok"
                : `Dönemin gönderilmemiş ${pending.length} satırı`
          }
          onClick={onSend}
        >
          {sheet.sheetUrl ? <Sheet /> : <FileSpreadsheet />}
          {sendLabel}
          {pending.length ? ` (${pending.length})` : ""}
        </Button>
      </div>
    </section>
  );
}

/** Denetimde bir konu ve etkilenen günler. */
function CheckGroup({
  title,
  hint,
  blocking,
  children,
}: {
  title: string;
  hint: string;
  blocking?: boolean;
  children: React.ReactNode;
}) {
  return (
    <li className="px-4 py-2.5">
      <div className="flex items-center gap-1.5 pb-1.5 text-xs">
        <TriangleAlert
          className={cn("size-3.5", blocking ? "text-destructive" : "text-amber-600 dark:text-amber-400")}
        />
        <span className="font-medium">{title}</span>
        <span className="text-muted-foreground">· {hint}</span>
      </div>
      <ul className="space-y-1 pl-5 text-xs">{children}</ul>
    </li>
  );
}
