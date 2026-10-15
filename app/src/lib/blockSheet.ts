import { api, type WorkBlock } from "../api";
import { friendlyError, notifyChanged, toast } from "./feedback";

/**
 * Takvim bloğunu zaman çizelgesine gönderme ve çıkarma. Yalnızca gönderilen blokların işi
 * çizelgede satır olur; satır blok değiştikçe değişir. Bildirimdeki "Geri al" tersini yapar.
 */

type BlockRef = Pick<WorkBlock, "start" | "end" | "projectId">;

/** Geri almayı çalıştırır; açık sayfalar yenilenir. */
async function revert(work: Promise<unknown>) {
  try {
    await work;
    notifyChanged();
    toast("Geri alındı", { tone: "success" });
  } catch (e) {
    toast(friendlyError(e), { tone: "error" });
  }
}

export async function sendBlock(b: BlockRef, onChanged: () => void) {
  await api.sendBlockToTimesheet(b.start, b.end, b.projectId);
  onChanged();
  toast("Zaman çizelgesine gönderildi", {
    tone: "success",
    action: { label: "Geri al", run: () => void revert(api.removeBlockFromTimesheet(b.start, b.end, b.projectId)) },
  });
}

export async function removeBlock(b: BlockRef, onChanged: () => void) {
  const r = await api.removeBlockFromTimesheet(b.start, b.end, b.projectId);
  onChanged();
  toast("Zaman çizelgesinden çıkarıldı", {
    ...(r.exported > 0 && {
      detail: `Dosyaya gönderilmiş ${r.exported} satır kaldı; Zaman çizelgesi sayfasından silebilirsin.`,
    }),
    action: { label: "Geri al", run: () => void revert(api.resendBlockToTimesheet(b.start, b.end, r.hidden)) },
  });
}
