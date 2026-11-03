import { useState } from "react";
import {
  ChevronRight,
  CircleCheck,
  ClipboardCheck,
  FileSpreadsheet,
  Loader2,
  Sheet,
  Sparkles,
  TriangleAlert,
  X,
} from "lucide-react";
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
import { type CloseReport, type Reconcile } from "../../lib/timesheet";
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
  recon,
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
  /** Takip edilen süreden dosyadaki saate döküm. */
  recon: Reconcile;
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

  const unassignedTotal = report.unassigned.reduce((s, u) => s + u.seconds, 0);
  const detailRows = report.details.reduce((s, d) => s + d.rows, 0);
  const blockingGroups = (report.stale.length > 0 ? 1 : 0) + (report.details.length > 0 ? 1 : 0);
  const warningGroups =
    (report.unassigned.length > 0 ? 1 : 0) +
    (report.meetings.length > 0 ? 1 : 0) +
    (report.hours.length > 0 ? 1 : 0) +
    (report.empty.length > 0 ? 1 : 0);
  const days_ = (n: number) => `${n} gün`;

  return (
    <section className="rounded-xl border bg-card shadow-xs" aria-label={`${title}: denetim`}>
      <div className="flex items-center gap-2 border-b px-4 py-2.5">
        <ClipboardCheck className="size-4 text-muted-foreground" />
        <span className="text-[13px] font-semibold">{title}</span>
        <span className="text-xs text-muted-foreground">
          {blockingGroups + warningGroups === 0
            ? "göndermeden önce denetim"
            : [
                blockingGroups > 0 && `${blockingGroups} konu gönderimi engelliyor`,
                warningGroups > 0 && `${warningGroups} uyarı`,
              ]
                .filter(Boolean)
                .join(" · ")}
        </span>
        <button
          aria-label="Kapat"
          className="-m-1 ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={onClose}
        >
          <X className="size-3.5" />
        </button>
      </div>
      <Breakdown r={recon} target={sheet.sheetUrl ? "Sheets" : "Excel"} />
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
        <ul className="divide-y border-t">
          {report.stale.length > 0 && (
            <CheckGroup
              blocking
              open
              title="Projesi sonradan değişen satırlar"
              summary={`${staleIds.length} satır`}
              hint="Bu satırlar kaydedildikten sonra işin bir kısmını raporda başka projeye aldın; satırdaki saat artık takiple uyuşmuyor. Güncelle'ye basınca saat düzelir. Güncellenmeden gönderilmez."
              action={
                <button className={FIX_LINK} onClick={run(() => refreshRows(staleIds))}>
                  {staleIds.length > 1 ? `hepsini güncelle (${staleIds.length})` : "güncelle"}
                </button>
              }
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
            </CheckGroup>
          )}
          {report.details.length > 0 && (
            <CheckGroup
              blocking
              open
              title="Açıklaması boş satırlar"
              summary={`${detailRows} satır · ${days_(report.details.length)}`}
              hint="Dosyaya açıklamasız satır yazılmaz; doldurulmadan gönderilmez."
            >
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
          {report.meetings.length > 0 && (
            <CheckGroup
              title="Projesi belli olmayan toplantılar"
              summary={days_(report.meetings.length)}
              hint="Proje seçilene kadar çizelgeye girmez. Seçilen proje serinin tüm tekrarlarına uygulanır."
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
              summary={days_(report.hours.length)}
              hint={`Günlük ${num.format(config.dayHours)} saat bekleniyor. Eksik ya da fazlaysa bilerek mi, bak.`}
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
              summary={days_(report.empty.length)}
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
          {report.unassigned.length > 0 && (
            <CheckGroup
              title="Projeye atanmamış süre"
              summary={`${formatDuration(unassignedTotal)} · ${days_(report.unassigned.length)}`}
              hint="Hiçbir projeye atanmadığı için çizelgeye girmez. İçinde bu firmanın işi varsa projeye ata; kişisel ya da başka işse geçebilirsin."
            >
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
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2 border-t px-4 py-2.5">
        <p className="mr-auto text-[11px] text-muted-foreground">
          {isBlocked
            ? "Açıklaması boş ya da projesi sonradan değişen satırlar düzeltilmeden gönderilmez."
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
              ? "Önce açıklaması boş ve projesi sonradan değişen satırları düzelt"
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

/** Takip edilen süreden dosyadaki saate döküm: farkın her kalemi ayrı satırda. */
function Breakdown({ r, target }: { r: Reconcile; target: string }) {
  const h = (v: number) => `${num.format(v)} sa`;
  const step = (v: number) => (Math.abs(v) < 0.005 ? null : `${v > 0 ? "+" : "−"}${num.format(Math.abs(v))} sa`);
  const lines: { label: string; hint: string; value: number; total?: boolean }[] = [
    {
      label: "Takip edilen",
      hint: "Bu çizelgeye bağlı projelerde takip edilen süre (bilgisayarda ve projesi belli toplantılarda).",
      value: r.tracked,
      total: true,
    },
    {
      label: "Çizelgeye gönderilmemiş bloklar",
      hint: 'Takvimde "Zaman çizelgesine gönder" denmemiş blokların süresi; satır olmaz. Firmanın işiyse takvimden gönder.',
      value: -r.unsentBlocks,
    },
    { label: "Gizlenen satırlar", hint: "Çizelgeden sildiğin satırların süresi.", value: -r.hidden },
    {
      label: "Kısa işler ve elle değişiklikler",
      hint: "5 dakikadan kısa işler, kaydedilmiş satırların yanında kalan birkaç dakika ve saatini elle değiştirdiğin satırlar.",
      value: r.other,
    },
    {
      label: "Satırların gerçek süresi",
      hint: "Çizelgedeki satırların yuvarlanmamış süresi.",
      value: r.actual,
      total: true,
    },
    {
      label: "Çeyrek saate yuvarlama",
      hint: "Firmaya giden saat her satırda en yakın 0,25'e yuvarlanır (0,83 → 0,75).",
      value: r.rounding,
    },
    { label: "Dosyaya elle girilen satırlar", hint: "Dosyada Kum dışında girilmiş satırlar.", value: r.outside },
    { label: "Çizelge toplamı", hint: "Sayfanın üstündeki toplam.", value: r.total, total: true },
    {
      label: "Henüz gönderilmemiş satırlar",
      hint: `Çizelgede olup ${target}'e daha aktarılmamış satırlar.`,
      value: -r.notSent,
    },
    { label: `${target} dosyasında`, hint: "Dosyada bu dönem için olması gereken saat.", value: r.file, total: true },
  ];
  return (
    <div className="px-4 py-3 text-xs">
      <div className="pb-1.5 font-medium">Takip edilen süreden {target}'e</div>
      <dl className="max-w-md">
        {lines.map((l) => {
          const text = l.total ? h(l.value) : step(l.value);
          if (text === null) return null;
          return (
            <div
              key={l.label}
              title={l.hint}
              className={cn(
                "flex items-baseline gap-2 py-0.5",
                l.total ? "border-t font-medium first:border-t-0" : "pl-3 text-muted-foreground",
              )}
            >
              <dt className="min-w-0 flex-1 truncate">{l.label}</dt>
              <dd className="tabular">{text}</dd>
            </div>
          );
        })}
      </dl>
      {r.unassigned >= 0.25 && (
        <p className="pt-1.5 text-[11px] text-muted-foreground">
          Ayrıca {h(r.unassigned)} projeye atanmamış süre var; hiçbir çizelgeye girmez (aşağıda).
        </p>
      )}
    </div>
  );
}

/** Denetimde bir konu: tek satırlık özet; tıklayınca etkilenen günler açılır. */
function CheckGroup({
  title,
  summary,
  hint,
  blocking,
  open: initial = false,
  action,
  children,
}: {
  title: string;
  summary: string;
  hint: string;
  blocking?: boolean;
  open?: boolean;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(initial);
  return (
    <li className="px-4 py-2.5">
      <div className="flex items-center gap-2 text-xs">
        <button
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          <ChevronRight
            className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
          />
          <TriangleAlert
            className={cn("size-3.5 shrink-0", blocking ? "text-destructive" : "text-amber-600 dark:text-amber-400")}
          />
          <span className="font-medium">{title}</span>
          <span className="text-muted-foreground tabular">· {summary}</span>
        </button>
        {action}
      </div>
      {open && (
        <>
          <p className="py-1.5 pl-5 text-[11px] text-muted-foreground">{hint}</p>
          <ul className="space-y-1 pl-5 text-xs">{children}</ul>
        </>
      )}
    </li>
  );
}
