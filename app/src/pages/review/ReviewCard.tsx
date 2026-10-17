import { useEffect, useMemo, useRef, useState } from "react";
import { AppWindow, CheckCircle2, ChevronDown, EyeOff, Globe, SkipForward, Sparkles, Wand2 } from "lucide-react";
import {
  formatDuration,
  NO_PROJECT,
  type RuleField,
  type Tag,
  type UnassignedGroup,
  type UnassignedItem,
} from "../../api";
import { ProjectSelect } from "../../components/ProjectSelect";
import { RulePreview } from "../../components/RulePreview";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import { readRecent } from "../../lib/projectOptions";
import { tagColor } from "../../lib/tags";
import { cn } from "../../lib/utils";

/** Gözden geçir kartı: bir site/uygulama grubu numaralı düğmeyle projeye, istenirse kuralıyla atanır. */

/** "Bundan sonra da otomatik yaz" son seçimi. */
const RULE_KEY = "kum.review.rule";

function savedRule(): boolean {
  try {
    return localStorage.getItem(RULE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Kartta listelenen başlık sayısı. */
const CARD_ITEMS = 4;
/** Kartta numaralı (1–4) proje düğmesi sayısı. */
const CHOICES = 4;

/**
 * Numaralı düğmeler: önce muhtemel proje (öncesinde/sonrasında çalışılan), sonra son seçilenler,
 * sonra dönemde en çok süre alan projeler.
 */
function pickChoices(projects: Tag[], likely: string | null, projectSeconds: Record<string, number>): Tag[] {
  const byId = new Map(projects.filter((p) => !p.archived).map((p) => [p.id, p]));
  const ranked = [...byId.values()].sort((a, b) => (projectSeconds[b.id] ?? 0) - (projectSeconds[a.id] ?? 0));
  const ids = [...(likely ? [likely] : []), ...readRecent(), ...ranked.map((p) => p.id)];
  return [...new Set(ids)].flatMap((id) => byId.get(id) ?? []).slice(0, CHOICES);
}

/** Kural türüne göre metin. */
const RULE_TEXT: Record<RuleField, string> = {
  domain: "Bu siteyi bundan sonra da bu projeye yaz",
  app: "Bu uygulamayı bundan sonra da bu projeye yaz",
  title: "Başlığında şu geçenleri hep bu projeye yaz",
};

export function ReviewCard({
  group,
  projects,
  tags,
  projectSeconds,
  onAssign,
  onAssignTitle,
  onSkip,
  onIgnore,
}: {
  group: UnassignedGroup;
  projects: Tag[];
  tags: Tag[];
  projectSeconds: Record<string, number>;
  onAssign: (projectId: string, rule: [RuleField, string] | null) => Promise<void>;
  onAssignTitle: (title: string, projectId: string, rule: [RuleField, string] | null) => void;
  onSkip: () => void;
  onIgnore: () => void;
}) {
  const likely = group.likelyProject;
  const choices = useMemo(
    () => pickChoices(projects, likely, projectSeconds),
    // Kart açıldığında sabitlenir: atama "son seçilenler"i değiştirince düğmeler kaymasın.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [group.key, projects],
  );
  const field: RuleField = group.kind === "site" ? "domain" : "app";
  const [rule, setRuleState] = useState(savedRule);
  const setRule = (v: boolean) => {
    setRuleState(v);
    try {
      localStorage.setItem(RULE_KEY, v ? "1" : "0");
    } catch {
      /* depolama kapalı */
    }
  };
  const [pattern, setPattern] = useState(group.pattern);
  const [previewFor, setPreviewFor] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const ruleOn = rule && pattern.trim().length > 0;

  async function choose(projectId: string) {
    if (busy) return;
    setBusy(true);
    try {
      await onAssign(projectId, ruleOn && projectId !== NO_PROJECT ? [field, pattern.trim()] : null);
    } finally {
      setBusy(false);
    }
  }

  // 1–4 projeye atar, S atlar. Uygulamanın 1/2/3 kısayollarından önce yakalanır.
  const keys = useRef<((e: KeyboardEvent) => void) | null>(null);
  keys.current = (e) => {
    if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest("input, textarea, select, [contenteditable], [role=dialog], [role=listbox], [role=menu]")) return;
    const n = Number(e.key);
    if (n >= 1 && n <= choices.length) void choose(choices[n - 1].id);
    else if (e.key === "s" || e.key === "S") onSkip();
    else return;
    e.preventDefault();
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current?.(e);
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const Icon = group.kind === "site" ? Globe : AppWindow;
  const items = expanded ? group.items : group.items.slice(0, CARD_ITEMS);
  const likelyTag = projects.find((p) => p.id === likely);
  const preview = previewFor ?? choices[0]?.id;

  return (
    <section className="animate-in space-y-4 rounded-2xl border bg-card p-5 shadow-sm fade-in-0 slide-in-from-right-3">
      <div className="flex items-center gap-3">
        <div
          className={cn(
            "grid size-10 shrink-0 place-items-center rounded-xl",
            group.kind === "site"
              ? "bg-sky-500/12 text-sky-600 dark:text-sky-400"
              : "bg-violet-500/12 text-violet-600 dark:text-violet-400",
          )}
        >
          <Icon className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] font-semibold" title={group.label}>
            {group.label}
          </div>
          <div className="truncate text-xs text-muted-foreground tabular">
            {group.kind === "site" ? group.appName : "Uygulama"} · {group.items.length + group.more} pencere
          </div>
        </div>
        <div className="shrink-0 text-[22px] font-semibold tracking-tight tabular">{formatDuration(group.seconds)}</div>
      </div>

      {likelyTag && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Sparkles className="size-3.5 text-primary" />
          Öncesinde ve sonrasında
          <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2 py-0.5 font-medium text-foreground">
            <i className="size-2 rounded-full" style={{ background: tagColor(likelyTag) }} aria-hidden />
            {likelyTag.name}
          </span>
          üzerinde çalışıyordun.
        </p>
      )}

      {(items.length > 0 || group.more > 0) && (
        <ul className="-mx-2 border-t pt-2">
          {items.map((item) => (
            <TitleRow
              key={item.title}
              item={item}
              projects={projects}
              tags={tags}
              likely={likely ?? undefined}
              onAssign={(p, r) => onAssignTitle(item.title, p, r)}
            />
          ))}
          {group.items.length > CARD_ITEMS && (
            <li>
              <button
                className="flex w-full items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                onClick={() => setExpanded(!expanded)}
              >
                <ChevronDown className={cn("size-3.5 transition-transform", expanded && "rotate-180")} />
                {expanded ? "Daha az göster" : `${group.items.length - CARD_ITEMS} başlık daha`}
              </button>
            </li>
          )}
          {group.more > 0 && (expanded || group.items.length <= CARD_ITEMS) && (
            <li className="px-2 py-1 text-[11px] text-muted-foreground">
              {group.items.length ? "ve " : ""}
              {group.more} kısa pencere (15 dk altı; kart atanınca hepsi dahil)
            </li>
          )}
        </ul>
      )}

      <div className="grid grid-cols-2 gap-2">
        {choices.map((p, i) => (
          <button
            key={p.id}
            disabled={busy}
            onClick={() => choose(p.id)}
            onMouseEnter={() => setPreviewFor(p.id)}
            onFocus={() => setPreviewFor(p.id)}
            className={cn(
              "flex h-10 min-w-0 items-center gap-2 rounded-xl border px-3 text-left text-[13px] font-medium outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring/50 disabled:opacity-50",
              p.id === likely && "border-primary/60 bg-primary/8 hover:bg-primary/12",
            )}
          >
            <i className="size-2.5 shrink-0 rounded-full" style={{ background: tagColor(p) }} aria-hidden />
            <span className="min-w-0 flex-1 truncate">{p.name}</span>
            <kbd className="rounded border border-b-2 bg-background px-1.5 font-mono text-[10px] text-muted-foreground">
              {i + 1}
            </kbd>
          </button>
        ))}
        {projects.length > choices.length && (
          <ProjectSelect
            value=""
            placeholder="Başka proje…"
            projects={projects}
            className="col-span-2 h-9"
            onChange={(id) => id && void choose(id)}
          />
        )}
      </div>

      <div className="space-y-2">
        <label className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={rule}
            onChange={(e) => setRule(e.target.checked)}
            className="size-3.5 accent-[var(--primary)]"
          />
          <span className="shrink-0">{RULE_TEXT[field]}</span>
          {rule &&
            (group.kind === "site" ? (
              <Input
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
                className="h-7 min-w-24 flex-1 text-xs"
                aria-label="Kural deseni"
              />
            ) : (
              <code className="truncate rounded bg-muted px-1.5 py-0.5 text-[11px]">{pattern}</code>
            ))}
        </label>
        {ruleOn && preview && <RulePreview tagId={preview} field={field} pattern={pattern} tags={tags} />}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3">
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            className="text-muted-foreground"
            disabled={busy}
            onClick={() => choose(NO_PROJECT)}
          >
            <CheckCircle2 /> Projesiz say
          </Button>
          <Button size="sm" variant="ghost" className="text-muted-foreground" disabled={busy} onClick={onIgnore}>
            <EyeOff /> Hiç gösterme
          </Button>
        </div>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={onSkip}
          title="Kartta bir daha gösterme, Atlananlar'a al"
        >
          <SkipForward /> Atla
          <kbd className="ml-1 font-mono text-[10px] text-muted-foreground">S</kbd>
        </Button>
      </div>
    </section>
  );
}

/**
 * Karttaki başlık: süre ve "Ayrı ata". Kart tümüyle atanmadan önce bir başlık başka projeye
 * (istenirse başlık kuralıyla) ayrılabilir.
 */
function TitleRow({
  item,
  projects,
  tags,
  likely,
  onAssign,
}: {
  item: UnassignedItem;
  projects: Tag[];
  tags: Tag[];
  likely?: string;
  onAssign: (projectId: string, rule: [RuleField, string] | null) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <li>
      <div className="group/title flex items-center gap-2 rounded-md px-2 py-1 hover:bg-accent/40">
        <span className={cn("min-w-0 flex-1 truncate text-xs", !item.title && "text-muted-foreground italic")}>
          {item.title || "(başlıksız)"}
        </span>
        {item.word && (
          <span className="hidden shrink-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary sm:inline-flex">
            <Wand2 className="size-3" /> {item.word}
          </span>
        )}
        <button
          className={cn(
            "shrink-0 rounded px-1.5 text-[11px] text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/50",
            !open && "opacity-0 group-hover/title:opacity-100 focus-visible:opacity-100",
          )}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? "Vazgeç" : "Ayrı ata"}
        </button>
        <span className="w-14 shrink-0 text-right text-[11px] text-muted-foreground tabular">
          {formatDuration(item.seconds)}
        </span>
      </div>
      {open && (
        <div className="animate-in px-2 pt-1 pb-2 fade-in-0 slide-in-from-top-1">
          <AssignForm
            projects={projects}
            tags={tags}
            field="title"
            pattern={item.word ?? item.title}
            editable
            defaultRule={!!item.word}
            ruleText={RULE_TEXT.title}
            initialProject={likely}
            submitLabel="Ata"
            onSubmit={async (p, rule) => {
              onAssign(p, rule);
              setOpen(false);
            }}
          />
        </div>
      )}
    </li>
  );
}

/** Proje seçimi, isteğe bağlı kural ve önizlemesi. */
function AssignForm({
  projects,
  tags,
  field,
  pattern: initialPattern,
  editable,
  defaultRule,
  ruleText,
  initialProject,
  submitLabel,
  onSubmit,
}: {
  projects: Tag[];
  tags: Tag[];
  field: RuleField;
  pattern: string;
  editable: boolean;
  defaultRule: boolean;
  ruleText: string;
  initialProject?: string;
  submitLabel: string;
  onSubmit: (projectId: string, rule: [RuleField, string] | null) => Promise<void>;
}) {
  const [project, setProject] = useState(initialProject ?? "");
  // Projeler atanmamış süreden sonra gelebilir: önerilen proje o zaman seçilsin.
  useEffect(() => {
    if (initialProject) setProject((p) => p || initialProject);
  }, [initialProject]);
  const [rule, setRule] = useState(defaultRule);
  const [pattern, setPattern] = useState(initialPattern);
  const [busy, setBusy] = useState(false);
  if (projects.length === 0) return null;
  const usable = !rule || pattern.trim().length > 0;
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <ProjectSelect value={project} onChange={setProject} projects={projects} className="w-48" />
        <label className="flex min-w-0 flex-1 items-center gap-2 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={rule}
            onChange={(e) => setRule(e.target.checked)}
            className="size-3.5 accent-[var(--primary)]"
          />
          <span className="shrink-0">{ruleText}</span>
          {rule &&
            (editable ? (
              <Input
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
                className="h-7 min-w-24 flex-1 text-xs"
                aria-label="Kural deseni"
              />
            ) : (
              <code className="truncate rounded bg-muted px-1.5 py-0.5 text-[11px]">{pattern}</code>
            ))}
        </label>
        <Button
          size="sm"
          disabled={!project || !usable || busy}
          onClick={async () => {
            setBusy(true);
            try {
              await onSubmit(project, rule ? [field, pattern.trim()] : null);
            } finally {
              setBusy(false);
            }
          }}
        >
          {submitLabel}
        </Button>
      </div>
      {rule && project && <RulePreview tagId={project} field={field} pattern={pattern} tags={tags} />}
    </div>
  );
}
