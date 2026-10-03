import { useEffect, useMemo, useState } from "react";
import { Check, Plus, Trash2, X } from "lucide-react";
import { api, type Rule, type RuleField, type Tag, type TagKind, type UsageTotal } from "../api";
import { ErrorText, Page } from "../components/settings";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "../components/ui/alert-dialog";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { nextColor } from "../lib/tags";
import { cn } from "../lib/utils";

const KIND_TITLE: Record<TagKind, string> = { category: "Kategoriler", project: "Projeler" };
const KIND_HINT: Record<TagKind, string> = {
  category:
    "Uygulamaları gruplar. Bir oturum tek kategoriye girer; başlık kuralları uygulama kurallarından önce gelir.",
  project: 'Pencere başlığında geçen bir kelimeyle uygulamalar arası işleri toplar (örn. "fintrack").',
};

export default function Categories() {
  const [tags, setTags] = useState<Tag[]>([]);
  const [rules, setRules] = useState<Rule[]>([]);
  const [apps, setApps] = useState<UsageTotal[]>([]);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    const t = await api.taxonomy();
    setTags(t.tags);
    setRules(t.rules);
  }

  useEffect(() => {
    load();
    api.knownApps().then(setApps);
  }, []);

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      await load();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <Page title="Kategoriler ve projeler">
      <ErrorText>{error}</ErrorText>
      {(["category", "project"] as TagKind[]).map((kind) => (
        <TagSection
          key={kind}
          kind={kind}
          tags={tags.filter((t) => t.kind === kind)}
          allTags={tags}
          rules={rules}
          apps={apps}
          run={run}
        />
      ))}
    </Page>
  );
}

/**
 * Başka işletim sisteminin uygulama kuralı mı? Hazır kategoriler hem macOS
 * bundle kimliklerini hem Windows exe adlarını içerir (senkronizasyonla iki
 * sistemde de geçerli); listede yalnızca bu sistemi ilgilendirenler gösterilir.
 */
function foreignRule(r: Rule): boolean {
  if (r.field !== "app") return false;
  const exe = /\.exe$/i.test(r.pattern);
  const platform = document.documentElement.dataset.platform;
  if (platform === "macos") return exe;
  if (platform === "windows") return !exe && r.pattern.includes(".");
  return false;
}

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

function TagSection({
  kind,
  tags,
  allTags,
  rules,
  apps,
  run,
}: {
  kind: TagKind;
  tags: Tag[];
  allTags: Tag[];
  rules: Rule[];
  apps: UsageTotal[];
  run: Run;
}) {
  const [name, setName] = useState("");
  const add = run(async () => {
    if (!name.trim()) return;
    await api.saveTag({ kind, name, color: nextColor(allTags) });
    setName("");
  });

  return (
    <section className="space-y-2">
      <div className="px-1">
        <h2 className="text-[13px] font-semibold">{KIND_TITLE[kind]}</h2>
        <p className="mt-0.5 text-xs text-muted-foreground">{KIND_HINT[kind]}</p>
      </div>
      <div className="divide-y rounded-xl border bg-card shadow-xs">
        {tags.map((t) => (
          <TagRow key={t.id} tag={t} rules={rules.filter((r) => r.tagId === t.id)} apps={apps} run={run} />
        ))}
        <form
          className="flex items-center gap-2 px-4 py-2.5"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <Plus className="size-4 text-muted-foreground" />
          <Input
            className="h-7 max-w-xs border-transparent bg-transparent px-1 shadow-none focus-visible:border-input dark:bg-transparent"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={kind === "category" ? "Yeni kategori ekle" : "Yeni proje ekle"}
          />
          {name.trim() && (
            <Button type="submit" size="sm">
              Ekle
            </Button>
          )}
        </form>
      </div>
    </section>
  );
}

function TagRow({ tag, rules, apps, run }: { tag: Tag; rules: Rule[]; apps: UsageTotal[]; run: Run }) {
  const [field, setField] = useState<RuleField>(tag.kind === "project" ? "title" : "app");
  const [pattern, setPattern] = useState("");
  const [name, setName] = useState(tag.name);
  // Başka cihazdan gelen yeniden adlandırma eski adla ezilmesin.
  useEffect(() => setName(tag.name), [tag.name]);
  const appNames = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);
  const shown = rules.filter((r) => !foreignRule(r));
  const hidden = rules.filter(foreignRule);

  const addRule = run(async () => {
    if (!pattern.trim()) return;
    await api.addRule(tag.id, field, pattern);
    setPattern("");
  });

  return (
    <div className="space-y-2.5 px-4 py-3">
      <div className="flex items-center gap-2">
        <ColorPicker value={tag.color} onChange={(color) => run(() => api.saveTag({ ...tag, color }))()} />
        <Input
          className="h-7 flex-1 border-transparent bg-transparent px-1.5 text-[13px] font-medium shadow-none hover:border-input focus-visible:border-input dark:bg-transparent"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== tag.name && run(() => api.saveTag({ ...tag, name }))()}
          aria-label="Ad"
        />
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground hover:text-destructive"
              aria-label="Sil"
            >
              <Trash2 className="size-3.5" />
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>“{tag.name}” silinsin mi?</AlertDialogTitle>
              <AlertDialogDescription>
                Kuralları da silinir. Geçmiş kayıtlar silinmez, kategorisiz görünür.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Vazgeç</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={run(() => api.deleteTag(tag.id))}
              >
                Sil
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
      <div className="flex flex-wrap items-center gap-1.5 pl-8">
        {shown.map((r) => (
          <span
            key={r.id}
            className="inline-flex h-6 items-center gap-1 rounded-md bg-secondary pr-0.5 pl-2 text-xs"
            title={r.pattern}
          >
            <span className="text-muted-foreground">{r.field === "app" ? "Uygulama" : "Başlıkta"}</span>
            <span className="max-w-48 truncate font-medium">
              {r.field === "app" ? (appNames.get(r.pattern) ?? r.pattern) : `“${r.pattern}”`}
            </span>
            <button
              className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
              onClick={run(() => api.deleteRule(r.id))}
              aria-label="Kuralı sil"
            >
              <X className="size-3" />
            </button>
          </span>
        ))}
        {hidden.length > 0 && (
          <span className="px-1 text-xs text-muted-foreground" title={hidden.map((r) => r.pattern).join("\n")}>
            +{hidden.length} başka sistem için
          </span>
        )}
        {rules.length === 0 && <span className="text-xs text-muted-foreground">Kural yok</span>}
      </div>
      <form
        className="flex flex-wrap items-center gap-1.5 pl-8"
        onSubmit={(e) => {
          e.preventDefault();
          addRule();
        }}
      >
        <Select
          value={field}
          onValueChange={(v) => {
            setField(v as RuleField);
            setPattern("");
          }}
        >
          <SelectTrigger size="sm" className="w-36" aria-label="Kural türü">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="app">Uygulama</SelectItem>
            <SelectItem value="title">Başlıkta geçen</SelectItem>
          </SelectContent>
        </Select>
        {field === "app" ? (
          <Select value={pattern} onValueChange={setPattern}>
            <SelectTrigger size="sm" className="w-52" aria-label="Uygulama">
              <SelectValue placeholder="Uygulama seç…" />
            </SelectTrigger>
            <SelectContent>
              {apps.map((a) => (
                <SelectItem key={a.key} value={a.key}>
                  {a.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <Input
            className="h-7 w-52 text-xs"
            value={pattern}
            onChange={(e) => setPattern(e.target.value)}
            placeholder="örn. fintrack"
          />
        )}
        <Button type="submit" variant="outline" size="sm" disabled={!pattern.trim()}>
          Kural ekle
        </Button>
      </form>
    </div>
  );
}

function ColorPicker({ value, onChange }: { value: number; onChange: (c: number) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          className="grid size-6 shrink-0 place-items-center rounded-full ring-offset-2 ring-offset-card transition-shadow hover:ring-2 hover:ring-border"
          aria-label="Renk seç"
        >
          <span className="size-3.5 rounded-full" style={{ background: `var(--c${value})` }} />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto p-2">
        <div className="grid grid-cols-4 gap-1.5">
          {[1, 2, 3, 4, 5, 6, 7, 8].map((c) => (
            <button
              key={c}
              className={cn(
                "grid size-7 place-items-center rounded-full text-white transition-transform hover:scale-110",
              )}
              style={{ background: `var(--c${c})` }}
              onClick={() => {
                setOpen(false);
                onChange(c);
              }}
              aria-label={`Renk ${c}`}
            >
              {c === value && <Check className="size-3.5" />}
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}
