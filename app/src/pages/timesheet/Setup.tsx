import { useState } from "react";
import { FileSpreadsheet, Sheet } from "lucide-react";
import { api, type Tag, type Timesheet as TimesheetInfo, type TimesheetConfig } from "../../api";
import { CalendarConnect, SheetConnect } from "../TimesheetSettings";
import { ErrorText } from "../../components/settings";
import { Button } from "../../components/ui/button";
import { ProjectSelect } from "../../components/ProjectSelect";
import { friendlyError } from "../../lib/feedback";

/** Çizelgede proje yoksa: hangi projelerin işi bu firmaya gidecek? */
export function ProjectsPrompt({
  config,
  sheet,
  projects,
  onSaved,
  onError,
}: {
  config: TimesheetConfig;
  sheet: TimesheetInfo;
  projects: Tag[];
  onSaved: () => void;
  onError: (e: string) => void;
}) {
  const taken = new Set(config.timesheets.flatMap((t) => t.projects.map((m) => m.projectId)));
  const free = projects.filter((p) => !taken.has(p.id));
  const add = async (projectId: string) => {
    try {
      await api.saveTimesheetConfig({
        ...config,
        timesheets: config.timesheets.map((t) =>
          t.id === sheet.id
            ? { ...t, projects: [...t.projects, { projectId, division: "", party: null, defaultDetails: null }] }
            : t,
        ),
      });
      onSaved();
    } catch (e) {
      onError(friendlyError(e));
    }
  };
  return (
    <section className="space-y-2 rounded-xl border border-dashed bg-card px-4 py-3 shadow-xs">
      <div className="text-[13px] font-semibold">Bu zaman çizelgesine hangi projeler gitsin?</div>
      <p className="text-xs text-muted-foreground">
        Yalnızca seçtiğin projelere atanan süre {sheet.company ? `${sheet.company} çizelgesinde` : "burada"} satır olur
        ve tablosuna gönderilir. Bir proje tek bir çizelgeye bağlanır.
      </p>
      {free.length > 0 ? (
        <ProjectSelect
          value=""
          projects={free}
          placeholder="Proje ekle…"
          className="w-60"
          aria-label="Çizelgeye proje ekle"
          onChange={add}
        />
      ) : (
        <p className="text-xs text-muted-foreground">Boşta proje yok; kenar çubuğundaki Projeler'den ekle.</p>
      )}
    </section>
  );
}

/**
 * İlk kurulum: kayıtların yazılacağı dosya (Excel ya da Google Sheets) ve isteğe bağlı Outlook
 * takvimi. Sonradan Ayarlar'dan değiştirilir; başka firmalar için yeni çizelge eklenir.
 */
export function Setup({ onDone }: { onDone: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [sheets, setSheets] = useState(false);
  const step = "space-y-2.5 rounded-xl border bg-card px-5 py-4 shadow-xs";
  return (
    <div className="mx-auto w-full max-w-xl space-y-4 px-6 pt-8 pb-10">
      <div className="space-y-1.5 px-1">
        <p className="text-sm text-muted-foreground">
          Projeye atanmış çalışma süren ve takvimindeki toplantılar günlük iş kayıtlarına dönüşür; gönderdiğin kayıtlar
          firmanın dosyasına aynı sütun ve biçimle eklenir. Her firmanın çizelgesine yalnızca ona bağladığın projelerin
          işi gider.
        </p>
      </div>
      <section className={step}>
        <h2 className="text-[13px] font-semibold">1. Kayıtlar nereye yazılsın?</h2>
        <p className="text-xs text-muted-foreground">
          Firma, danışman, birimler (projeler) ve geçmiş açıklamalar seçtiğin dosyadan alınır.
        </p>
        {sheets ? (
          <SheetConnect timesheetId={null} onDone={onDone} onCancel={() => setSheets(false)} />
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  const path = await api.pickTimesheetFile();
                  if (path) {
                    await api.importTimesheetTemplate(null, path);
                    onDone();
                  }
                } catch (e) {
                  setError(friendlyError(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <FileSpreadsheet /> Excel dosyası seç
            </Button>
            <Button variant="outline" onClick={() => setSheets(true)}>
              <Sheet /> Google Sheets'e bağla
            </Button>
          </div>
        )}
        <ErrorText>{error}</ErrorText>
      </section>
      <section className={step}>
        <h2 className="text-[13px] font-semibold">
          2. Outlook takvimi <span className="font-normal text-muted-foreground">(isteğe bağlı)</span>
        </h2>
        <p className="text-xs text-muted-foreground">Toplantılar da kayıtlara girer.</p>
        <CalendarConnect />
      </section>
      <p className="px-1 text-xs text-muted-foreground">
        İkisini de sonra <b>Ayarlar</b>'dan değiştirebilirsin.
      </p>
    </div>
  );
}
