import { useState } from "react";
import { AppWindow, CheckCircle2, ChevronDown, Globe, SkipForward, Wand2 } from "lucide-react";
import { formatDuration, type RuleSuggestion, type Tag, type UnassignedGroup } from "../../api";
import { ProjectSelect } from "../../components/ProjectSelect";
import { RulePreview } from "../../components/RulePreview";
import { Button } from "../../components/ui/button";
import { ruleReason, ruleSubject } from "../../lib/ruleSuggestions";
import { tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";

/** Gözden geçir sayfasının kart dışındaki parçaları: özet, kural önerileri, atlananlar, gizlenenler. */

/** Tek satır özet: kalan atanmamış süre ve projelerdeki pay. */
export function Progress({ left, worked, share }: { left: number; worked: number; share: number }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="shrink-0">
        <b className="font-semibold text-foreground tabular">{formatDuration(left)}</b> atanmamış
      </span>
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-gradient-to-r from-[var(--brand-1)] to-[var(--brand-2)] transition-[width] duration-500 ease-out"
          style={{ width: `${share * 100}%` }}
        />
      </div>
      <span className="shrink-0 tabular">{worked > 0 ? `%${Math.round(share * 100)} projede` : "çalışma yok"}</span>
    </div>
  );
}

export function ReviewSkeleton() {
  return (
    <div className="space-y-3">
      <div className="skeleton h-4 rounded-full" />
      <div className="skeleton h-[320px] rounded-2xl" />
    </div>
  );
}

/**
 * Elle atamalardan öğrenilen kural önerileri: her biri gerekçesi ve önizlemesiyle; "Kural ekle"
 * kuralı ekler (geri alınabilir), "Önerme" bir daha göstermez.
 */
export function RuleSuggestions({
  suggestions,
  projects,
  tags,
  onAdd,
  onDismiss,
}: {
  suggestions: RuleSuggestion[];
  projects: Tag[];
  tags: Tag[];
  onAdd: (s: RuleSuggestion) => Promise<void>;
  onDismiss: (s: RuleSuggestion) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <section className="space-y-2">
      <button
        className="mx-auto flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <Wand2 className="size-3.5 text-primary" />
        <b className="font-semibold">{suggestions.length} kural önerisi</b>: sık elle atadığın süre kendiliğinden düşsün
        <ChevronDown className={cn("size-3.5 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <ul className="space-y-2">
          {suggestions.map((s) => (
            <RuleSuggestionCard
              key={s.key}
              suggestion={s}
              project={projects.find((p) => p.id === s.projectId)}
              tags={tags}
              onAdd={() => onAdd(s)}
              onDismiss={() => onDismiss(s)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function RuleSuggestionCard({
  suggestion: s,
  project,
  tags,
  onAdd,
  onDismiss,
}: {
  suggestion: RuleSuggestion;
  project?: Tag;
  tags: Tag[];
  onAdd: () => Promise<void>;
  onDismiss: () => void;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <li className="animate-in space-y-2 rounded-xl border bg-card px-4 py-3 shadow-xs fade-in-0 slide-in-from-bottom-1">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5 text-[13px] font-semibold">
            <span>{ruleSubject(s)}</span>
            <span className="text-muted-foreground">→</span>
            <span className="inline-flex items-center gap-1.5">
              {project && <i className="size-2 rounded-full" style={{ background: tagColor(project) }} aria-hidden />}
              {project?.name ?? s.projectName}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] text-muted-foreground">{ruleReason(s)}</p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button size="sm" variant="ghost" className="text-muted-foreground" onClick={onDismiss} disabled={busy}>
            Önerme
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onAdd();
              } finally {
                setBusy(false);
              }
            }}
          >
            Kural ekle
          </Button>
        </div>
      </div>
      <RulePreview tagId={s.projectId} field={s.field} pattern={s.pattern} tags={tags} />
    </li>
  );
}

/** Atlanan gruplar: kartta gelmez; buradan projeye atanır ya da karta geri alınır. */
export function SkippedList({
  groups,
  projects,
  onAssign,
  onRestore,
}: {
  groups: UnassignedGroup[];
  projects: Tag[];
  onAssign: (group: UnassignedGroup, projectId: string) => void;
  onRestore: (key: string) => void;
}) {
  return (
    <section className="space-y-2">
      <div className="flex items-center gap-2 px-1">
        <SkipForward className="size-4 text-muted-foreground" />
        <h2 className="text-[13px] font-semibold">Atlananlar</h2>
        <span className="text-xs text-muted-foreground tabular">
          {formatDuration(groups.reduce((a, g) => a + g.seconds, 0))}
        </span>
      </div>
      <ul className="divide-y rounded-xl border bg-card shadow-xs">
        {groups.map((g) => {
          const Icon = g.kind === "site" ? Globe : AppWindow;
          return (
            <li key={g.key} className="flex items-center gap-3 px-4 py-2">
              <Icon className="size-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <div className="truncate text-xs font-medium" title={g.label}>
                  {g.label}
                </div>
                <div className="truncate text-[11px] text-muted-foreground">
                  {g.kind === "site" ? g.appName : "Uygulama"} · {g.items.length + g.more} pencere
                </div>
              </div>
              <span className="w-16 shrink-0 text-right text-[11px] text-muted-foreground tabular">
                {formatDuration(g.seconds)}
              </span>
              <ProjectSelect
                value=""
                placeholder="Projeye ata…"
                projects={projects}
                className="w-44"
                onChange={(id) => id && onAssign(g, id)}
              />
              <Button size="sm" variant="ghost" className="h-7 px-2 text-[11px]" onClick={() => onRestore(g.key)}>
                Karta al
              </Button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

export function AllDone({ skipped }: { skipped: number }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed px-6 py-12 text-center">
      <div className="grid size-14 place-items-center rounded-2xl bg-gradient-to-br from-emerald-400 to-teal-500 text-white shadow-lg shadow-emerald-500/25">
        <CheckCircle2 className="size-7" />
      </div>
      <div>
        <p className="text-[15px] font-semibold">{skipped ? "Sıradaki kart kalmadı" : "Her şey bir projede"}</p>
        <p className="mt-1 text-[13px] text-muted-foreground">
          {skipped ? `Atladığın ${skipped} grup aşağıda.` : "Bu dönemde atanmamış süre yok."}
        </p>
      </div>
    </div>
  );
}

export function IgnoredList({ keys, onShow }: { keys: string[]; onShow: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="px-1 text-xs text-muted-foreground">
      <button
        className="rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {keys.length} grup listede gösterilmiyor {open ? "▴" : "▾"}
      </button>
      {open && (
        <ul className="mt-2 flex flex-wrap gap-1.5">
          {keys.map((k) => (
            <li key={k} className="inline-flex items-center gap-1 rounded-md border bg-card py-0.5 pr-0.5 pl-2">
              <span>{k.replace(/^(site|app):/, "")}</span>
              <Button size="sm" variant="ghost" className="h-5 px-1.5 text-[11px]" onClick={() => onShow(k)}>
                Göster
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
