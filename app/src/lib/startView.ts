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
