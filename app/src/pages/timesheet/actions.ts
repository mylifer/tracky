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
  )
    .catch((e) => toast(friendlyError(e), { tone: "error" }))
    .finally(notifyChanged);
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
            ])
              .catch((e) => toast(friendlyError(e), { tone: "error" }))
              .finally(notifyChanged);
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
          .catch((e) => toast(friendlyError(e), { tone: "error" }))
          .finally(notifyChanged),
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
