import type { WindowSpan } from "../api";

/** Bloktaki bir pencere: aynı başlık ve projede geçen toplam süre. */
export type BlockWindow = {
  title: string;
  domain: string | null;
  projectId: string | null;
  seconds: number;
};

/** Bloktaki bir uygulama ve içinde zaman geçirilen pencereler (uzundan kısaya). */
export type BlockApp = {
  appId: string;
  appName: string;
  seconds: number;
  windows: BlockWindow[];
};

/**
 * Takvim bloğunun `[start, end)` aralığına düşen pencereler, uygulamaya göre gruplanmış.
 * Pencere aralıkları bloğa kırpılır; aynı başlık farklı projelere atanmışsa ayrı satır olur
 * (hangi kısmın hangi projeye yazıldığı görünsün).
 */
export function blockWindows(spans: WindowSpan[], start: string, end: string): BlockApp[] {
  const from = Date.parse(start);
  const to = Date.parse(end);
  const apps = new Map<string, BlockApp & { byKey: Map<string, BlockWindow> }>();
  for (const w of spans) {
    const ms = Math.min(Date.parse(w.end), to) - Math.max(Date.parse(w.start), from);
    if (ms <= 0) continue;
    let app = apps.get(w.appId);
    if (!app) {
      app = { appId: w.appId, appName: w.appName, seconds: 0, windows: [], byKey: new Map() };
      apps.set(w.appId, app);
    }
    app.seconds += ms / 1000;
    const key = `${w.title}\u0000${w.projectId ?? ""}`;
    let item = app.byKey.get(key);
    if (!item) {
      item = { title: w.title, domain: w.domain, projectId: w.projectId, seconds: 0 };
      app.byKey.set(key, item);
    }
    item.seconds += ms / 1000;
  }
  return [...apps.values()]
    .map(({ byKey, ...app }) => ({
      ...app,
      seconds: Math.round(app.seconds),
      windows: [...byKey.values()]
        .map((w) => ({ ...w, seconds: Math.round(w.seconds) }))
        .filter((w) => w.seconds > 0)
        .sort((a, b) => b.seconds - a.seconds || a.title.localeCompare(b.title, "tr")),
    }))
    .filter((a) => a.seconds > 0)
    .sort((a, b) => b.seconds - a.seconds || a.appName.localeCompare(b.appName, "tr"));
}
