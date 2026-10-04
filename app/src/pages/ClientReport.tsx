import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronLeft, ChevronRight, FileSpreadsheet, Printer } from "lucide-react";
import { api, type Client, type ClientReport as Report, type ReportSource } from "../api";
import { ErrorText } from "../components/settings";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { addMonths, formatMonth, isoDate, parseIsoDate, startOfMonth, today } from "../lib/dates";
import { friendlyError, toast, useChanged } from "../lib/feedback";
import { NO_CLIENT } from "../lib/tags";
import { cn } from "../lib/utils";

/** Müşteri seçicide "hepsi" (Radix Select boş değer kabul etmez). */
const ALL = "all";

const hoursFmt = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 0, maximumFractionDigits: 2 });
const totalFmt = new Intl.NumberFormat("tr-TR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const weekdayFmt = new Intl.DateTimeFormat("tr-TR", { weekday: "narrow" });

/** Boş gün boş kalır (tablo okunur olsun). */
const cell = (h: number) => (h > 0.004 ? hoursFmt.format(h) : "");

const SOURCE_LABEL: Record<ReportSource, string> = {
  timesheet: "Zaman çizelgesi satırları",
  tracked: "Takip edilen süre",
};

/**
 * Aylık müşteri raporu: projelerin gün gün saatleri (müşteri onayı ve fatura için). Ayda zaman
 * çizelgesi kaydı varsa firmaya yazılan (yuvarlanmış) saatler, yoksa takip edilen süre; kullanıcı
 * ikisi arasında geçebilir. Excel'e aktarılır ya da yazdırılır (yazdırırken yalnızca rapor).
 */
export default function ClientReport() {
  const [month, setMonth] = useState(isoDate(startOfMonth(today())));
  const [client, setClient] = useState<string>(ALL);
  // `null`: kendisi seçer (zaman çizelgesi varsa o).
  const [source, setSource] = useState<ReportSource | null>(null);
  const [clients, setClients] = useState<Client[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [version, setVersion] = useState(0);
  const reload = useCallback(() => setVersion((v) => v + 1), []);
  useChanged(reload);

  useEffect(() => {
    api.taxonomy().then(
      (t) => setClients(t.clients),
      () => {},
    );
  }, []);

  useEffect(() => {
    let live = true;
    api.clientReport(month, client === ALL ? null : client, source).then(
      (r) => {
        if (!live) return;
        setReport(r);
        setError(null);
      },
      (e) => live && setError(friendlyError(e)),
    );
    return () => {
      live = false;
    };
  }, [month, client, source, version]);

  function changeMonth(n: number) {
    setMonth(isoDate(addMonths(parseIsoDate(month), n)));
    setSource(null);
  }

  async function exportXlsx() {
    if (!report) return;
    setExporting(true);
    try {
      const path = await api.exportClientReport(month, client === ALL ? null : client, report.source);
      if (path) toast(`Excel'e aktarıldı: ${path.split(/[\\/]/).pop()}`, { tone: "success" });
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    } finally {
      setExporting(false);
    }
  }

  const monthDate = parseIsoDate(month);
  const atCurrent = month === isoDate(startOfMonth(today()));
  const clientName = client === ALL ? "Tüm müşteriler" : (clients.find((c) => c.id === client)?.name ?? "");
  const title = `${clientName} · ${capitalize(formatMonth(monthDate))}`;
  const shown = report?.source ?? source ?? "timesheet";

  return (
    <div className="mx-auto w-full max-w-6xl space-y-5 px-6 pt-2 pb-10">
      <h1 className="sr-only">Müşteri raporu</h1>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" onClick={() => changeMonth(-1)} aria-label="Önceki ay">
            <ChevronLeft />
          </Button>
          <span className="min-w-28 text-center text-[13px] font-medium tabular">
            {capitalize(formatMonth(monthDate))}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => changeMonth(1)}
            disabled={atCurrent}
            aria-label="Sonraki ay"
          >
            <ChevronRight />
          </Button>
        </div>
        <Select
          value={client}
          onValueChange={(v) => {
            setClient(v);
            setSource(null);
          }}
        >
          <SelectTrigger size="sm" className="w-44" aria-label="Müşteri">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>Tüm müşteriler</SelectItem>
            {clients.map((c) => (
              <SelectItem key={c.id} value={c.id}>
                {c.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Tabs value={shown} onValueChange={(v) => setSource(v as ReportSource)}>
          <TabsList aria-label="Kaynak">
            <TabsTrigger
              value="timesheet"
              className="px-3"
              disabled={!!report && !report.timesheetAvailable}
              title={report && !report.timesheetAvailable ? "Bu ay için zaman çizelgesi kaydı yok" : undefined}
            >
              {SOURCE_LABEL.timesheet}
            </TabsTrigger>
            <TabsTrigger value="tracked" className="px-3">
              {SOURCE_LABEL.tracked}
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="flex-1" />
        <Button size="sm" variant="outline" onClick={() => window.print()} disabled={!report?.rows.length}>
          <Printer /> Yazdır / PDF
        </Button>
        <Button size="sm" onClick={exportXlsx} disabled={!report?.rows.length || exporting}>
          <FileSpreadsheet /> Excel'e aktar
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>

      {report && report.rows.length === 0 ? (
        <p className="rounded-xl border border-dashed px-5 py-4 text-sm text-muted-foreground">
          {report.source === "timesheet"
            ? "Bu ay için onaylanmış zaman çizelgesi kaydı yok."
            : "Bu ay projeye düşen süre yok. Projeleri bir müşteriye bağlamak için Müşteriler sayfasını kullan."}
        </p>
      ) : report ? (
        <section className="space-y-2">
          <div className="overflow-x-auto rounded-xl border bg-card shadow-xs">
            <Matrix report={report} />
          </div>
          <p className="px-1 text-xs text-muted-foreground">
            {report.source === "timesheet"
              ? "Saatler onaylanmış zaman çizelgesi kayıtlarından (firmaya yazılan, yuvarlanmış saat)."
              : "Saatler projeye düşen takip edilen süreden; bilgisayarlar arası çakışmalar bir kez sayılır."}
          </p>
        </section>
      ) : null}

      {report &&
        report.rows.length > 0 &&
        createPortal(
          <div className="kum-print">
            <h1>{title}</h1>
            <p>Kaynak: {SOURCE_LABEL[report.source].toLowerCase()}</p>
            <Matrix report={report} print />
          </div>,
          document.body,
        )}
      <style>{PRINT_CSS}</style>
    </div>
  );
}

/** Yazdırırken uygulama gizlenir, yalnızca `body` altındaki rapor kopyası basılır. */
const PRINT_CSS = `
.kum-print { display: none; }
@media print {
  @page { size: A4 landscape; margin: 10mm; }
  html, body { background: #fff !important; color: #000 !important; height: auto !important; overflow: visible !important; }
  #root { display: none !important; }
  .kum-print { display: block; font: 9px/1.3 -apple-system, "Segoe UI", sans-serif; }
  .kum-print * { color: #000 !important; background: transparent !important; }
  .kum-print h1 { font-size: 14px; font-weight: 600; margin: 0 0 2px; }
  .kum-print p { margin: 0 0 8px; }
  .kum-print table { width: 100%; border-collapse: collapse; }
  .kum-print th, .kum-print td { border: 1px solid #ccc; padding: 2px 3px; }
  .kum-print .weekend { background: #f1f1f1 !important; }
  .kum-print tfoot td { font-weight: 600; background: #ece6da !important; }
  .kum-print i { display: none; }
}
`;

function Matrix({ report, print }: { report: Report; print?: boolean }) {
  const days = report.days.map(parseIsoDate);
  const weekend = days.map((d) => d.getDay() === 0 || d.getDay() === 6);
  const showClient = report.rows.some((r) => r.client);
  const num = "px-1 text-right tabular";
  return (
    <table className={cn("w-full border-collapse text-xs", !print && "min-w-max")}>
      <thead>
        <tr className="border-b text-[11px] text-muted-foreground">
          <th className={cn("px-3 py-2 text-left font-medium", !print && "sticky left-0 z-10 min-w-44 bg-card")}>
            Proje
          </th>
          {days.map((d, i) => (
            <th
              key={i}
              className={cn("w-8 px-1 py-1.5 text-center font-medium", weekend[i] && "weekend bg-muted/50")}
              title={d.toLocaleDateString("tr-TR", { weekday: "long", day: "numeric", month: "long" })}
            >
              <div className="tabular text-foreground/80">{d.getDate()}</div>
              <div className="text-[10px]">{weekdayFmt.format(d)}</div>
            </th>
          ))}
          <th className="px-3 py-2 text-right font-medium">Toplam</th>
        </tr>
      </thead>
      <tbody className="divide-y">
        {report.rows.map((r) => (
          <tr key={r.projectId || `?${r.project}`} className="hover:bg-muted/30">
            <td className={cn("px-3 py-1.5", !print && "sticky left-0 z-10 bg-card")}>
              <span className="flex min-w-0 items-center gap-2">
                <i className="size-2 shrink-0 rounded-full" style={{ background: `var(--c${r.color})` }} />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{r.project}</span>
                  {showClient && (
                    <span className="block truncate text-[11px] text-muted-foreground">{r.client ?? NO_CLIENT}</span>
                  )}
                </span>
              </span>
            </td>
            {r.hours.map((h, i) => (
              <td key={i} className={cn(num, "py-1.5", weekend[i] && "weekend bg-muted/50")}>
                {cell(h)}
              </td>
            ))}
            <td className="px-3 py-1.5 text-right font-semibold tabular">{totalFmt.format(r.total)}</td>
          </tr>
        ))}
      </tbody>
      <tfoot>
        <tr className="border-t bg-muted/40 font-semibold">
          <td className={cn("px-3 py-2", !print && "sticky left-0 z-10 bg-muted")}>Toplam</td>
          {report.dayTotals.map((h, i) => (
            <td key={i} className={cn(num, "py-2")}>
              {cell(h)}
            </td>
          ))}
          <td className="px-3 py-2 text-right tabular">{totalFmt.format(report.total)}</td>
        </tr>
      </tfoot>
    </table>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toLocaleUpperCase("tr-TR") + s.slice(1);
}
