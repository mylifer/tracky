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

/** Bloğun içinde verilen projelere düşen süre ve bu sürenin aralıkları (unix ms). */
export type ProjectShare = { seconds: number; ranges: [number, number][] };

/**
 * Bloğun `[start, end)` aralığında `ids` projelerine yazılmış pencereler. Aralıklar, arada başka
 * projenin (ya da projesiz) penceresi yoksa birleşir: aralığa yapılan düzenleme yalnızca bu
 * projenin süresine dokunsun, kayıtsız boşluklar da tek aralıkta kalsın.
 */
export function projectShare(spans: WindowSpan[], start: string, end: string, ids: Set<string>): ProjectShare {
  const from = Date.parse(start);
  const to = Date.parse(end);
  const inside = spans
    .map((w) => ({
      a: Math.max(Date.parse(w.start), from),
      b: Math.min(Date.parse(w.end), to),
      own: w.projectId !== null && ids.has(w.projectId),
    }))
    .filter((w) => w.b > w.a)
    .sort((x, y) => x.a - y.a);
  const ranges: [number, number][] = [];
  let seconds = 0;
  // Son aralıktan sonra başka projenin penceresi geldi mi: geldiyse yeni aralık başlar.
  let broken = true;
  for (const w of inside) {
    if (!w.own) {
      broken = true;
      continue;
    }
    seconds += (w.b - w.a) / 1000;
    const last = ranges[ranges.length - 1];
    if (last && !broken) last[1] = Math.max(last[1], w.b);
    else ranges.push([w.a, w.b]);
    broken = false;
  }
  return { seconds: Math.round(seconds), ranges };
}
