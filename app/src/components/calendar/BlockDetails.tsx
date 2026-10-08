import { useMemo, useState } from "react";
import { Briefcase, ChevronDown, FolderInput, Monitor } from "lucide-react";
import type { BlockDevice, Tag, WindowSpan, WorkBlock } from "../../api";
import { api, formatDuration, NO_PROJECT } from "../../api";
import { blockWindows, type BlockApp, type BlockWindow } from "../../lib/blockWindows";
import { detailRows, onlyOther, type DetailRow } from "../../lib/minorWindows";
import { friendlyError, toast, undoable } from "../../lib/feedback";
import { UNASSIGNED_MIN } from "../../lib/timesheet";
import { formatTime } from "../../lib/dates";
import { UNASSIGNED, UNCATEGORIZED, tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";
import { DeviceIcon } from "../DeviceIcon";
import { Badge } from "../ui/badge";
import { AppIcon, SiteIcon } from "../AppIcon";
import { BlockActions, useEdit } from "../SessionEdit";
import { ProjectSelect } from "../ProjectSelect";
import { short } from "./grid";

/** Bloğun ayrıntı kartı: kategori, süre, uygulama yüzdeleri. */
export function BlockDetails({
  block,
  tags,
  windows,
}: {
  block: WorkBlock;
  tags: Map<string, Tag>;
  /** Raporun pencere aralıkları; verilirse uygulamaların altında pencereler de listelenir. */
  windows?: WindowSpan[];
}) {
  const edit = useEdit();
  const apps = useMemo(() => (windows ? blockWindows(windows, block.start, block.end) : null), [windows, block]);
  const tag = block.categoryId ? tags.get(block.categoryId) : undefined;
  const project = block.projectId ? tags.get(block.projectId) : undefined;
  const color = tagColor(tag);
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="outline" className="gap-1.5">
          <i className="size-2 rounded-full" style={{ background: color }} />
          {tag?.name ?? UNCATEGORIZED}
        </Badge>
        {project && (
          <Badge variant="outline" className="min-w-0 gap-1.5" title="Proje">
            <Briefcase className="shrink-0" style={{ color: tagColor(project) }} />
            <span className="truncate">{project.name}</span>
          </Badge>
        )}
      </div>
      <div>
        <div className="flex items-center justify-between gap-3">
          <span className="flex min-w-0 items-center gap-2">
            {block.topApps[0] && <AppIcon appId={block.topApps[0].appId} name={block.topApps[0].appName} size={20} />}
            <strong className="truncate text-sm">{block.topApps[0]?.appName ?? tag?.name ?? "Çalışma"}</strong>
          </span>
          <span className="text-sm font-semibold tabular">{formatDuration(block.activeSeconds)}</span>
        </div>
        <p className="mt-0.5 text-xs text-muted-foreground tabular">
          {formatTime(new Date(block.start))} – {formatTime(new Date(block.end))}
          {block.switches > 0 && ` · ${block.switches} uygulama geçişi`}
        </p>
      </div>
      {block.devices && block.devices.length > 0 && <BlockDevices devices={block.devices} />}
      {apps && apps.length > 0 ? (
        <BlockApps block={block} apps={apps} tags={tags} color={color} />
      ) : (
        <ul className="space-y-1.5">
          {block.topApps.map((a) => {
            const pct = block.activeSeconds ? Math.round((a.seconds / block.activeSeconds) * 100) : 0;
            return (
              <li key={a.appName} className="grid grid-cols-[34px_1fr_auto] items-center gap-2 text-xs">
                <span className="text-muted-foreground tabular">%{pct}</span>
                <span className="min-w-0">
                  <span className="flex min-w-0 items-center gap-1.5">
                    <AppIcon appId={a.appId} name={a.appName} size={14} />
                    <span className="truncate">{a.appName}</span>
                  </span>
                  <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                    <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
                  </span>
                </span>
                <span className="text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
              </li>
            );
          })}
        </ul>
      )}
      {edit && (
        <BlockActions
          start={block.start}
          end={block.end}
          categoryId={block.categoryId}
          projectId={block.projectId}
          categories={edit.categories}
          projects={edit.projects}
          onChanged={edit.onChanged}
        />
      )}
    </div>
  );
}

/** Bilgisayar başına renk: aynı bilgisayar her blokta aynı renkte görünsün. */
function deviceColor(id: string) {
  let h = 0;
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return `var(--c${(h % 8) + 1})`;
}

/** Bloğun hangi bilgisayardan geldiği; birden çok bilgisayar varsa dağılımıyla. */
function BlockDevices({ devices }: { devices: BlockDevice[] }) {
  const total = devices.reduce((n, d) => n + d.seconds, 0);
  if (devices.length === 1)
    return (
      <div className="flex items-center gap-2 rounded-md bg-muted/60 px-2.5 py-1.5 text-xs">
        <DeviceIcon model={devices[0].model} className="size-4 text-muted-foreground" />
        <span className="text-muted-foreground">Bilgisayar</span>
        <span className="ml-auto truncate font-medium">{devices[0].name}</span>
      </div>
    );
  return (
    <div className="space-y-1.5 rounded-md bg-muted/60 px-2.5 py-2 text-xs">
      <div className="flex items-center gap-2 text-muted-foreground">
        <Monitor className="size-3.5 shrink-0" aria-hidden />
        Bilgisayarlar
      </div>
      <div className="flex h-1.5 gap-0.5 overflow-hidden rounded-full" aria-hidden>
        {devices.map((d) => (
          <span key={d.id} style={{ flex: d.seconds, background: deviceColor(d.id) }} />
        ))}
      </div>
      <ul className="space-y-0.5">
        {devices.map((d) => (
          <li key={d.id} className="flex items-center gap-2">
            <i className="size-2 shrink-0 rounded-full" style={{ background: deviceColor(d.id) }} aria-hidden />
            <DeviceIcon model={d.model} className="size-4 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate">{d.name}</span>
            <span className="text-muted-foreground tabular">
              %{total ? Math.round((d.seconds / total) * 100) : 0} · {formatDuration(d.seconds)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Bir uygulamada gösterilen ilk pencere sayısı; gerisi "daha fazla" ile açılır. */
const FIRST_WINDOWS = 5;
/** Seçicide "kurallara bırak" (elle atamayı kaldır). */
const AUTO = "__otomatik__";

/**
 * Bloktaki uygulamalar ve her birinde zaman geçirilen pencereler: hangi işin yapıldığı (ve
 * hangi projeye yazıldığı) başlıktan anlaşılsın; pencere tek başına projeye atanabilir.
 */
function BlockApps({
  block,
  apps,
  tags,
  color,
}: {
  block: WorkBlock;
  apps: BlockApp[];
  tags: Map<string, Tag>;
  color: string;
}) {
  const total = apps.reduce((n, a) => n + a.seconds, 0);
  return (
    <div className="-mr-2 max-h-80 space-y-3 overflow-y-auto pr-2">
      {apps.map((a) => {
        const pct = total ? Math.round((a.seconds / total) * 100) : 0;
        return (
          <section key={a.appId}>
            <div className="grid grid-cols-[34px_1fr_auto] items-center gap-2 text-xs">
              <span className="text-muted-foreground tabular">%{pct}</span>
              <span className="min-w-0">
                <span className="flex min-w-0 items-center gap-1.5">
                  <AppIcon appId={a.appId} name={a.appName} size={16} />
                  <span className="truncate font-medium">{a.appName}</span>
                </span>
                <span className="mt-1 block h-1 overflow-hidden rounded-full bg-muted">
                  <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
                </span>
              </span>
              <span className="text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
            </div>
            <AppWindows block={block} app={a} tags={tags} />
          </section>
        );
      })}
    </div>
  );
}

function AppWindows({ block, app, tags }: { block: WorkBlock; app: BlockApp; tags: Map<string, Tag> }) {
  const [all, setAll] = useState(false);
  const rows = useMemo(() => windowRows(app.windows, tags), [app.windows, tags]);
  // Yalnızca "Diğer" kalan uygulamada liste, üstteki süreyi tekrarlamaktan öteye geçmez.
  if (onlyOther(rows)) return null;
  const shown = all ? rows : rows.slice(0, FIRST_WINDOWS);
  const hidden = rows.length - shown.length;
  return (
    <ul className="mt-1.5 ml-[42px] space-y-0.5 border-l pl-2">
      {shown.map((r) =>
        r.kind === "item" ? (
          <WindowRow
            key={`${r.item.title}\u0000${r.item.projectId ?? ""}`}
            block={block}
            appId={app.appId}
            w={r.item}
            tags={tags}
          />
        ) : (
          <GroupRow key={r.key} row={r} tags={tags} />
        ),
      )}
      {(hidden > 0 || all) && rows.length > FIRST_WINDOWS && (
        <li>
          <button
            className="flex items-center gap-1 rounded py-0.5 text-[11px] text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
            aria-expanded={all}
            onClick={() => setAll(!all)}
          >
            <ChevronDown className={cn("size-3 transition-transform", all && "rotate-180")} />
            {all ? "Daha az göster" : `${hidden} satır daha`}
          </button>
        </li>
      )}
    </ul>
  );
}

/**
 * Bloktaki pencereler ayrıntı satırlarına: kısa olanlar aynı projede (projesizse aynı sitede)
 * birlikte anlamlıysa toplanır, gerisi "Diğer".
 */
function windowRows(windows: BlockWindow[], tags: Map<string, Tag>): DetailRow<BlockWindow>[] {
  return detailRows(
    windows,
    (w) => w.seconds,
    (w) => {
      const project = w.projectId && w.projectId !== NO_PROJECT ? tags.get(w.projectId) : undefined;
      if (project) return { key: `p:${project.id}`, label: project.name, projectId: project.id };
      if (w.domain) return { key: `d:${w.domain}`, label: w.domain, domain: w.domain };
      return null;
    },
  );
}

/** Kısa pencerelerin toplam satırı: adları tek tek gösterilmez, yalnızca ortak bağ ve süre. */
function GroupRow({ row, tags }: { row: Extract<DetailRow<BlockWindow>, { kind: "group" }>; tags: Map<string, Tag> }) {
  const project = row.bond?.projectId ? tags.get(row.bond.projectId) : undefined;
  return (
    <li className="flex items-center gap-2 rounded-md px-1.5 py-1 text-xs text-muted-foreground">
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        {project ? (
          <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} aria-hidden />
        ) : (
          row.bond?.domain && <SiteIcon domain={row.bond.domain} />
        )}
        <span className="truncate">{row.label}</span>
      </span>
      <span className="shrink-0 text-[11px] tabular">{formatDuration(row.seconds)}</span>
    </li>
  );
}

/** Pencere satırı: başlık, site, proje ve süre; "Ata" ile yalnızca bu pencerenin süresi atanır. */
function WindowRow({
  block,
  appId,
  w,
  tags,
}: {
  block: WorkBlock;
  appId: string;
  w: BlockWindow;
  tags: Map<string, Tag>;
}) {
  const edit = useEdit();
  const [open, setOpen] = useState(false);
  const project = w.projectId ? tags.get(w.projectId) : undefined;
  // Atama yalnızca zaman çizelgesine girecek kadar uzun (≥ 15 dk) atanmamış pencerede önerilir.
  const suggest = !project && w.seconds >= UNASSIGNED_MIN;
  const title = w.title || "(başlıksız)";

  async function assign(v: string) {
    if (!edit) return;
    const id = v === AUTO ? null : v;
    const name =
      id === null
        ? "kurallara bırakıldı"
        : id === NO_PROJECT
          ? "projesiz sayıldı"
          : `→ ${tags.get(id)?.name ?? "proje"}`;
    try {
      await undoable(api.assignWindow(block.start, block.end, appId, w.title, id), `“${short(title)}” ${name}`);
      setOpen(false);
      edit.onChanged();
    } catch (e) {
      toast(friendlyError(e), { tone: "error" });
    }
  }

  return (
    <li className="group/w rounded-md px-1.5 py-1 hover:bg-accent/40">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className={cn("truncate text-xs", !w.title && "text-muted-foreground italic")} title={title}>
            {title}
          </div>
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            {w.domain && (
              <span className="flex min-w-0 items-center gap-1 truncate">
                <SiteIcon domain={w.domain} />
                <span className="truncate">{w.domain}</span>
              </span>
            )}
            {w.domain && <span aria-hidden>·</span>}
            {project ? (
              <span className="flex min-w-0 items-center gap-1">
                <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(project) }} aria-hidden />
                <span className="truncate">{project.name}</span>
              </span>
            ) : (
              <span className={cn(suggest && "font-medium text-foreground")}>{UNASSIGNED}</span>
            )}
          </div>
        </div>
        <span className="shrink-0 pt-px text-[11px] text-muted-foreground tabular">{formatDuration(w.seconds)}</span>
        {edit && edit.projects.length > 0 && (
          <button
            className={cn(
              "flex h-6 shrink-0 items-center gap-1 rounded-md px-1.5 text-[11px] focus-visible:outline-2 focus-visible:outline-ring",
              suggest
                ? "border bg-background font-medium hover:bg-accent"
                : "text-muted-foreground opacity-0 group-hover/w:opacity-100 hover:bg-accent hover:text-foreground focus-visible:opacity-100",
              open && "opacity-100",
            )}
            aria-expanded={open}
            aria-label={`“${title}” penceresini projeye ata`}
            title="Bu blokta bu pencerede geçen süreyi projeye ata"
            onClick={() => setOpen(!open)}
          >
            <FolderInput className="size-3" aria-hidden />
            {suggest && "Ata"}
          </button>
        )}
      </div>
      {open && edit && (
        <ProjectSelect
          className="mt-1.5 w-full"
          value=""
          projects={edit.projects}
          placeholder="Projeye ata…"
          extra={[
            { value: NO_PROJECT, label: "Projesiz" },
            ...(w.projectId ? [{ value: AUTO, label: "Otomatik (kurallara göre)" }] : []),
          ]}
          aria-label={`“${title}” projesi`}
          onChange={assign}
        />
      )}
    </li>
  );
}
