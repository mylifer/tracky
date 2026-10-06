import { api, type EntryView } from "../../api";
import { addDays, isoDate, parseIsoDate } from "../../lib/dates";
import { friendlyError, notifyChanged, toast } from "../../lib/feedback";
import { copyDetails } from "../../lib/timesheet";
import { toRef } from "./shared";

/** Açıklama kopyalarken geriye bakılan gün sayısı (hafta sonu ve izin günlerini aşsın). */
const COPY_LOOKBACK = 7;

type Saved = { before: EntryView; id: string };

/**
 * Kaydedilen satırları geri alır: önceden canlı olan satır silinir (yeniden takipten gelir),
 * kaydedilmiş satır önceki haline döner.
 */
function undoSaves(saved: Saved[]) {
  Promise.all(
    saved.map(({ before, id }) =>
      before.id ? api.saveTimesheetEntry(before.id, before) : api.deleteTimesheetEntry(id),
    ),
  ).then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" }));
}

/**
 * Satırlara açıklama yazar (canlı satır kaydedilir); yazılanlar için "Geri al"lı bildirim.
 * Yarıda hata olursa o ana kadar yazılanlar kalır ve yine geri alınabilir.
 */
async function writeDetails(changes: { entry: EntryView; details: string }[], message: (n: number) => string) {
  const saved: Saved[] = [];
  try {
    for (const c of changes)
      saved.push({ before: c.entry, id: await api.saveTimesheetEntry(c.entry.id, { ...c.entry, details: c.details }) });
  } finally {
    if (saved.length > 0)
      toast(message(saved.length), { tone: "success", action: { label: "Geri al", run: () => undoSaves(saved) } });
  }
}

/** Önceki günlerin aynı projedeki açıklamalarını günün boş açıklamalı satırlarına yazar ([copyDetails]). */
export async function copyPreviousDetails(sheetId: string, date: string) {
  const [[day], prior] = await Promise.all([
    api.timesheetDays(sheetId, date, 1),
    api.timesheetDays(sheetId, isoDate(addDays(parseIsoDate(date), -COPY_LOOKBACK)), COPY_LOOKBACK),
  ]);
  prior.reverse();
  const changes = copyDetails(day.entries, prior);
  if (changes.length === 0) {
    toast("Önceki günlerde bu projelere yazılmış açıklama yok");
    return;
  }
  await writeDetails(changes, (n) => `${n} satıra önceki günlerden açıklama yazıldı`);
}

/**
 * Günlerin açıklamalarını yapay zekâyla (Claude) yazar: gün başına bir istek, sırayla. Varsayılan
 * olarak yalnızca boş ya da otomatik gelen (başlıklardan, hazır açıklamadan) açıklamalar yazılır,
 * elle yazılana dokunulmaz; `rewrite` ise günün aktarılmamış bütün satırları. Yazılanlar hemen
 * kaydedilir, bildirimden hepsi birden geri alınır. Bir gün hata verirse sonraki günlere
 * geçilmez; o ana kadar yazılanlar kalır.
 */
export async function aiWriteDays(
  sheetId: string,
  dates: string[],
  rewrite: boolean,
  onProgress?: (done: number) => void,
) {
  const saved: Saved[] = [];
  let failure: unknown = null;
  for (const [i, date] of dates.entries()) {
    try {
      const [day] = await api.timesheetDays(sheetId, date, 1);
      if (!day.entries.some((e) => !e.exported)) continue;
      for (const w of await api.aiWriteDetails(sheetId, date, rewrite)) {
        const entry = day.entries.find((e) => e.key === w.key);
        if (!entry || entry.details === w.details) continue;
        saved.push({ before: entry, id: await api.saveTimesheetEntry(entry.id, { ...entry, details: w.details }) });
      }
    } catch (e) {
      failure = e;
      break;
    } finally {
      onProgress?.(i + 1);
    }
  }
  if (saved.length > 0)
    toast(`${saved.length} satıra yapay zekâyla açıklama yazıldı`, {
      tone: "success",
      action: { label: "Geri al", run: () => undoSaves(saved) },
    });
  else if (!failure) toast("Yazılacak açıklama yok: boş ya da otomatik açıklamalı, aktarılmamış satır kalmadı");
  if (failure) throw failure;
}

/** Satırları gizler (siler); bildirimden geri alınır. */
export async function dismissRows(rows: EntryView[]) {
  const done: { row: EntryView; id: string }[] = [];
  try {
    for (const row of rows) done.push({ row, id: await api.dismissTimesheetEntry(row.id, row) });
  } finally {
    if (done.length > 0)
      toast(done.length === 1 ? "Satır silindi" : `${done.length} satır silindi`, {
        action: {
          label: "Geri al",
          run: () => {
            const saved = done.filter((d) => d.row.id).map((d) => d.id);
            // Canlı satır gizlenince kaydedilmişti: silinince yeniden takipten gelir.
            const live = done.filter((d) => !d.row.id).map((d) => d.id);
            Promise.all([
              saved.length ? api.undismissTimesheetEntries(saved) : null,
              ...live.map((id) => api.deleteTimesheetEntry(id)),
            ]).then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" }));
          },
        },
      });
  }
}

/** Satırları birleştirir; bildirimden geri alınır. */
export async function mergeRows(rows: EntryView[]) {
  const merged = await api.mergeTimesheetEntries(rows.map(toRef));
  toast(`${rows.length} satır birleştirildi`, {
    tone: "success",
    action: {
      label: "Geri al",
      run: () =>
        api
          .unmergeTimesheetEntries(merged.id, merged.removed)
          .then(notifyChanged, (e) => toast(friendlyError(e), { tone: "error" })),
    },
  });
}

/** Takipte değişen satırları günceller (projede işi kalmayan satır silinir). */
export async function refreshRows(ids: string[]) {
  const removed = await api.refreshTimesheetEntries(ids);
  const updated = ids.length - removed;
  toast(
    [updated ? `${updated} satır güncellendi` : "", removed ? `${removed} satırın projede işi kalmadı, silindi` : ""]
      .filter(Boolean)
      .join("; "),
    { tone: "success" },
  );
}
