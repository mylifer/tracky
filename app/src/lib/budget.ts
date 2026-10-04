import type { BudgetUsage } from "../api";

/** Bütçenin bu oranında uyarılır (çekirdekteki `BUDGET_WARN_RATIO`). */
export const BUDGET_WARN = 0.8;

const daysFormat = new Intl.NumberFormat("tr-TR", { maximumFractionDigits: 1 });

/** Saniyeyi adam-güne çevirip yazar: "12,5". */
export function formatDays(seconds: number, dayHours: number): string {
  return daysFormat.format(seconds / ((dayHours || 8) * 3600));
}

/** Harcanan / bütçe (1 = doldu). */
export function budgetRatio(u: BudgetUsage): number {
  return u.budgetSeconds > 0 ? u.usedSeconds / u.budgetSeconds : 0;
}

/** Durum: bütçe dolduysa "over", %80'i geçtiyse "near". */
export function budgetState(u: BudgetUsage): "ok" | "near" | "over" {
  const r = budgetRatio(u);
  return r >= 1 ? "over" : r >= BUDGET_WARN ? "near" : "ok";
}

/**
 * Haftalık sürelerden kalan bütçe eğrisi (saniye): ilk öğe dönemin başında, sonrakiler her
 * haftanın sonunda (son hafta sürüyor: şimdi). `used` bugüne kadarki toplam; dönemden önceki
 * harcama ilk öğeye yansır. Bütçe aşıldıysa değerler eksiye düşer.
 */
export function burnDown(budgetSeconds: number, usedSeconds: number, weekly: number[]): number[] {
  const out = [budgetSeconds - usedSeconds];
  for (let i = weekly.length - 1; i >= 0; i--) out.unshift(out[0] + weekly[i]);
  return out;
}

/**
 * Bu hızla bütçe kaç haftada biter? Hız, süren hafta hariç son 4 haftanın (çalışılan
 * haftalar) ortalaması; hiç çalışılmadıysa ya da bütçe bittiyse `null`.
 */
export function weeksLeft(remainingSeconds: number, weekly: number[]): number | null {
  if (remainingSeconds <= 0) return null;
  const done = weekly.slice(0, -1).slice(-4);
  const avg = done.reduce((a, b) => a + b, 0) / (done.length || 1);
  return avg > 0 ? remainingSeconds / avg : null;
}
