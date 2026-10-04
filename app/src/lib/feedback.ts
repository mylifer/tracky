import { useEffect, useRef, useSyncExternalStore } from "react";
import { api, type Edited, type RuleSuggestion } from "../api";
import { ruleSentence } from "./ruleSuggestions";

/**
 * Geri bildirim: ekranın altındaki kısa bildirimler ("Silindi · Geri al"), veri değişince
 * açık sayfaların yenilenmesi ve arka uç hatalarının okunur hale getirilmesi.
 */

export type Toast = {
  id: number;
  message: string;
  tone: "default" | "success" | "error";
  /** Arka uçtaki geri alma numarası; varsa "Geri al" düğmesi görünür. */
  undo?: number;
  action?: { label: string; run: () => void };
  /** İletinin altında küçük ikinci satır (örn. önerilen kural). */
  detail?: string;
};

let toasts: Toast[] = [];
let seq = 0;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

/** En çok bu kadar bildirim üst üste durur. */
const MAX_TOASTS = 3;

export function toast(message: string, opts: Partial<Omit<Toast, "id" | "message">> = {}): number {
  const id = ++seq;
  toasts = [...toasts, { id, message, tone: opts.tone ?? "default", ...opts }].slice(-MAX_TOASTS);
  emit();
  return id;
}

export function dismissToast(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export function useToasts(): Toast[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => toasts,
  );
}

export const CHANGED = "kum:changed";

/** Kayıtlar değişti (geri alma, gözden geçirme…): açık rapor ve listeler yenilensin. */
export function notifyChanged() {
  window.dispatchEvent(new Event(CHANGED));
}

/** Kayıtlar başka bir yerden değişince `cb` çağrılır. */
export function useChanged(cb: () => void) {
  const latest = useRef(cb);
  useEffect(() => {
    latest.current = cb;
  });
  useEffect(() => {
    const on = () => latest.current();
    window.addEventListener(CHANGED, on);
    return () => window.removeEventListener(CHANGED, on);
  }, []);
}

/** Düzenlemeyi geri alır ve sonucu bildirir. */
export async function undo(id: number) {
  try {
    await api.undo(id);
    notifyChanged();
    toast("Geri alındı", { tone: "success" });
  } catch (e) {
    toast(friendlyError(e), { tone: "error" });
  }
}

/**
 * Geri alınabilir bir düzenlemeyi çalıştırır; bittiğinde "Geri al" düğmeli bildirim gösterir.
 * Arka uç geri alma numarasını ya doğrudan ya da `Edited` içinde döndürür. Elle atama bir
 * alışkanlığa dönüştüyse (`Edited.suggestion`) bildirimde "Kural yap" da çıkar.
 */
export async function undoable<T extends Edited | number>(work: Promise<T>, message: string): Promise<T> {
  const result = await work;
  if (typeof result === "number") {
    toast(message, { undo: result });
    return result;
  }
  const s = result.suggestion;
  toast(message, {
    undo: result.undo,
    ...(s && {
      detail: `Öneri: ${ruleSentence(s)}`,
      action: { label: "Kural yap", run: () => void addSuggestedRule(s) },
    }),
  });
  return result;
}

/** Önerilen kuralı ekler (geri alınabilir); açık sayfalar yenilenir. */
export async function addSuggestedRule(s: RuleSuggestion): Promise<boolean> {
  try {
    await undoable(api.addRule(s.projectId, s.field, s.pattern), `Kural eklendi: ${ruleSentence(s)}`);
    notifyChanged();
    return true;
  } catch (e) {
    toast(friendlyError(e), { tone: "error" });
    return false;
  }
}

/** Bilinen arka uç hata kalıpları → kullanıcının anlayacağı cümle. */
const KNOWN: [RegExp, string][] = [
  [
    /database is locked|veritabanı kilitli/i,
    "Kayıtlar şu an başka bir işlemle güncelleniyor. Birkaç saniye sonra tekrar dene.",
  ],
  [/^veritabanı hatası/i, "Kayıtlara erişilemedi. Uygulamayı yeniden başlatmayı dene."],
  [/etiket bulunamadı/i, "Seçilen proje ya da kategori artık yok; sayfayı yenile."],
  [
    /(error sending request|dns|timed out|connection refused|network)/i,
    "Bağlantı kurulamadı. İnternet bağlantını kontrol et.",
  ],
  [/permission denied|operation not permitted/i, "İzin gerekli: dosyaya ya da klasöre erişilemedi."],
];

/** Arka uç hatasını kısa ve anlaşılır hale getirir ("geçersiz kayıt: bu aralıkta…" → "Bu aralıkta…"). */
export function friendlyError(e: unknown): string {
  const raw = String(e instanceof Error ? e.message : e).trim();
  for (const [re, text] of KNOWN) if (re.test(raw)) return text;
  const m = raw.replace(/^(Error: )?(geçersiz kayıt|ayar okunamadı): /i, "");
  if (!m) return "Bir şeyler ters gitti. Tekrar dene.";
  const s = m.charAt(0).toLocaleUpperCase("tr") + m.slice(1);
  return /[.!?…]$/.test(s) ? s : `${s}.`;
}
