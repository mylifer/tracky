import type { Client, Tag } from "../api";
import { NO_CLIENT } from "./tags";

/** Son seçilen projeler (bu cihazda). */
const RECENT_KEY = "kum.recentProjects";
export const RECENT_MAX = 5;
/** Bu kadar proje olunca seçicinin yanında süzme alanı çıkar. */
export const FILTER_MIN = 12;

export type ProjectGroup = { label: string | null; projects: Tag[] };

export function readRecent(): string[] {
  try {
    const list: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((x): x is string => typeof x === "string").slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

/** Seçilen projeyi listenin başına alır (en çok `RECENT_MAX`). */
export function withRecent(list: string[], id: string): string[] {
  return [id, ...list.filter((x) => x !== id)].slice(0, RECENT_MAX);
}

export function rememberRecent(id: string) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(withRecent(readRecent(), id)));
  } catch {
    /* depolama kapalıysa hatırlanmaz */
  }
}

const fold = (s: string) => s.toLocaleLowerCase("tr-TR");

/**
 * Seçicinin seçenek grupları: en üstte son kullanılanlar (süzme yokken ve liste kısa değilse),
 * sonra müşteriye göre (adına göre sıralı) projeler, en sonda müşterisiz olanlar. Müşteri
 * yoksa tek, başlıksız grup. `filter` proje ya da müşteri adında aranır; `keep` (seçili
 * proje) süzmede de listede kalır ki seçicinin gösterdiği değer kaybolmasın.
 */
export function projectGroups(
  projects: Tag[],
  opts: {
    clients?: Client[];
    projectClients?: Record<string, string>;
    recent?: string[];
    filter?: string;
    keep?: string;
  } = {},
): ProjectGroup[] {
  const clients = opts.clients ?? [];
  const owner = opts.projectClients ?? {};
  const clientOf = (p: Tag) => clients.find((c) => c.id === owner[p.id]);
  const q = fold(opts.filter?.trim() ?? "");
  const shown = q
    ? projects.filter(
        (p) => p.id === opts.keep || fold(p.name).includes(q) || fold(clientOf(p)?.name ?? "").includes(q),
      )
    : projects;
  const out: ProjectGroup[] = [];
  if (!q && projects.length > RECENT_MAX) {
    const recent = (opts.recent ?? []).flatMap((id) => projects.filter((p) => p.id === id));
    if (recent.length > 0) out.push({ label: "Son kullanılanlar", projects: recent });
  }
  if (!shown.some(clientOf)) {
    if (shown.length > 0) out.push({ label: out.length ? "Tüm projeler" : null, projects: shown });
    return out;
  }
  const byName = [...clients].sort((a, b) => a.name.localeCompare(b.name, "tr"));
  for (const c of byName) {
    const list = shown.filter((p) => owner[p.id] === c.id);
    if (list.length > 0) out.push({ label: c.name, projects: list });
  }
  const rest = shown.filter((p) => !clientOf(p));
  if (rest.length > 0) out.push({ label: NO_CLIENT, projects: rest });
  return out;
}
