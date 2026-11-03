/** Uygulama açılınca gösterilen sayfa: Gün raporu ya da Bugün. */
export type StartView = "day" | "home";

const KEY = "kum.startView";

/** Kayıtlı açılış sayfası; bu cihazda hatırlanır. */
export function savedStartView(): StartView {
  try {
    return localStorage.getItem(KEY) === "home" ? "home" : "day";
  } catch {
    return "day";
  }
}

export function saveStartView(v: StartView) {
  try {
    localStorage.setItem(KEY, v);
  } catch {
    /* depolama kapalıysa yalnızca bu oturumda */
  }
}

/** Gün/Hafta/Ay raporunda son bakılan tarih; uygulama yeniden açılınca da korunur. */
export function savedReportDate(mode: "day" | "week" | "month"): string | null {
  try {
    const v = localStorage.getItem(`kum.report.${mode}`);
    if (v && /^\d{4}-\d{2}-\d{2}$/.test(v)) return v;
  } catch {
    /* depolama kapalıysa bugün */
  }
  return null;
}

export function saveReportDate(mode: "day" | "week" | "month", iso: string) {
  try {
    localStorage.setItem(`kum.report.${mode}`, iso);
  } catch {
    /* hatırlanmasa da olur */
  }
}
