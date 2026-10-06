import { useEffect, useMemo, useState } from "react";
import { Archive, ArchiveRestore, Check, ChevronDown, Plus, Trash2, X } from "lucide-react";
import {
  api,
  formatDuration,
  type BudgetUsage,
  type Client,
  type Rule,
  type RuleField,
  type Tag,
  type TagKind,
  type UsageTotal,
} from "../api";
import { RulePreview } from "./RulePreview";
import { BudgetField, BudgetMeter } from "./Budget";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { friendlyError, undoable } from "../lib/feedback";
import { clientColor, NO_CLIENT, nextColor, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

/*
 * Proje ve kategori düzenleme parçaları: ekleme alanı, müşteri seçici, ad/renk, kural listeleri.
 * Kategoriler sayfası ve Projeler sayfasının proje detayı kullanır.
 */

/** Sayfaya göre metinler: projeler başlıktaki sözcüklerle, kategoriler uygulamalarla çalışır. */
export const TEXT = {
  project: {
    title: "Projeler",
    intro:
      "Proje, uygulamalar arası bir iştir: pencere başlığında projenin sözcüğü geçen her şey (Figma dosyası, tarayıcı sekmesi, kod klasörü) o projeye yazılır.",
    add: "Yeni proje adı",
    addHint: "Proje adı, başlıkta aranan ilk sözcük olur; sonra başka sözcükler de ekleyebilirsin.",
    primary: "title" as RuleField,
    empty:
      "Henüz proje yok. Üstten bir ad yaz (örn. müşteri ya da iş adı); başlığında geçen pencereler o projeye sayılır.",
    deleteHint: "Kuralları da silinir. Geçmiş kayıtlar silinmez, projesiz görünür.",
  },
  category: {
    title: "Kategoriler",
    intro:
      "Kategori, zamanın ne tür işe gittiğini gösterir (Tasarım, İletişim…). Her oturum tek kategoriye girer; web sitesi ve başlık kuralları uygulama kurallarından önce gelir.",
    add: "Yeni kategori adı",
    addHint: "Ekledikten sonra kategoriye uygulamaları ata.",
    primary: "app" as RuleField,
    empty: "Henüz kategori yok. Üstten bir ad yaz, sonra uygulamaları ata.",
    deleteHint: "Kuralları da silinir. Geçmiş kayıtlar silinmez, kategorisiz görünür.",
  },
} satisfies Record<TagKind, unknown>;

export type Run = (f: () => Promise<unknown>) => () => Promise<void>;

/** Projenin müşterisi: yerel açılır liste (kartın içinde ayrı katman açmaz). */
export function ClientSelect({
  clients,
  value,
  onChange,
  className,
}: {
  clients: Client[];
  value: string | null;
  onChange: (id: string | null) => void;
  className?: string;
}) {
  return (
    <span className={cn("relative inline-flex items-center", className)}>
      <i
        className="pointer-events-none absolute left-2.5 size-2 rounded-full"
        style={{ background: clientColor(clients, value) }}
        aria-hidden
      />
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
        className="h-8 w-full appearance-none rounded-md border bg-transparent pr-7 pl-6 text-xs hover:bg-accent dark:bg-input/30"
        aria-label="Müşteri"
      >
        <option value="">{NO_CLIENT}</option>
        {clients.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" aria-hidden />
    </span>
  );
}

/** En üstteki ekleme alanı. */
export function AddTag({
  kind,
  allTags,
  clients,
  onAdded,
  onError,
}: {
  kind: TagKind;
  allTags: Tag[];
  clients: Client[];
  onAdded: (id: string) => void;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  // Yeni projenin müşterisi; art arda aynı müşteriye proje eklenirken seçili kalır.
  const [client, setClient] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const text = TEXT[kind];
  const same = allTags.find(
    (t) => t.kind === kind && t.name.toLocaleLowerCase("tr") === name.trim().toLocaleLowerCase("tr"),
  );
  const exists = !!same;
  return (
    <form
      className="space-y-1.5 rounded-xl border bg-card px-4 py-3 shadow-xs"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim() || exists) return;
        setBusy(true);
        try {
          const tag = await api.saveTag({ kind, name: name.trim(), color: nextColor(allTags) });
          // Kuralsız proje hiç süre toplamaz: adı, başlıkta aranan sözcük olarak eklenir. Kural
          // eklenemezse proje de kaldırılır; yoksa ad "zaten var" olur ve yeniden denenemez.
          if (kind === "project")
            await api.addRule(tag.id, "title", tag.name).catch(async (err) => {
              await api.deleteTag(tag.id).catch(() => {});
              throw err;
            });
          if (kind === "project" && client) await api.setProjectClient(tag.id, client);
          setName("");
          onAdded(tag.id);
        } catch (err) {
          onError(friendlyError(err));
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="flex items-center gap-2">
        <Input
          className="h-8 flex-1 text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={text.add}
          aria-label={text.add}
        />
        {kind === "project" && clients.length > 0 && (
          <ClientSelect clients={clients} value={client} onChange={setClient} className="w-40" />
        )}
        <Button type="submit" size="sm" disabled={!name.trim() || exists || busy}>
          <Plus /> Ekle
        </Button>
      </div>
      <p className={cn("text-[11px]", exists ? "text-destructive" : "text-muted-foreground")}>
        {exists
          ? `“${name.trim()}” zaten var${same?.archived ? " (arşivde; aşağıdaki Arşiv'den geri alabilirsin)" : ""}.`
          : text.addHint}
      </p>
    </form>
  );
}

/**
 * Başka işletim sisteminin uygulama kuralı mı? Hazır kategoriler hem macOS
 * bundle kimliklerini hem Windows exe adlarını içerir (senkronizasyonla iki
 * sistemde de geçerli); listede yalnızca bu sistemi ilgilendirenler gösterilir.
 */
export function foreignRule(r: Rule): boolean {
  if (r.field !== "app") return false;
  const exe = /\.exe$/i.test(r.pattern);
  const platform = document.documentElement.dataset.platform;
  if (platform === "macos") return exe;
  if (platform === "windows") return !exe && r.pattern.includes(".");
  return false;
}

/** Kural özeti: "2 sözcük · 1 site · 1 uygulama". */
export function ruleSummary(rules: Rule[]) {
  const words = rules.filter((r) => r.field === "title").length;
  const sites = rules.filter((r) => r.field === "domain").length;
  const apps = rules.filter((r) => r.field === "app" && !foreignRule(r)).length;
  const parts = [words && `${words} sözcük`, sites && `${sites} site`, apps && `${apps} uygulama`].filter(Boolean);
  return parts.length ? parts.join(" · ") : "Kural yok: süre toplamaz";
}

/**
 * Etiketin düzenleyicisi: ad ve renk, (projede) müşteri ve bütçe, kurallar, arşivleme ve
 * silme. Projeler sayfasında proje detayında, Kategoriler sayfasında açılan satırda.
 */
export function TagEditor({
  tag,
  rules,
  apps,
  clients,
  clientId,
  budget,
  dayHours,
  run,
  allTags,
}: {
  tag: Tag;
  rules: Rule[];
  apps: UsageTotal[];
  clients: Client[];
  clientId: string | null;
  /** Projenin sözleşme bütçesi ve harcanan (tüm zamanlar). */
  budget?: BudgetUsage;
  dayHours: number;
  run: Run;
  allTags: Tag[];
}) {
  const text = TEXT[tag.kind];
  return (
    <div className="space-y-4">
      <NameAndColor tag={tag} run={run} />
      {tag.kind === "project" && (
        <div className="space-y-1.5">
          <div className="text-[11px] font-medium text-muted-foreground">Müşteri</div>
          {clients.length > 0 ? (
            <ClientSelect
              clients={clients}
              value={clientId}
              onChange={(id) => run(() => api.setProjectClient(tag.id, id))()}
              className="w-56"
            />
          ) : (
            <p className="text-xs text-muted-foreground">Müşteri yok; kenar çubuğundaki Müşteriler'den ekle.</p>
          )}
        </div>
      )}
      {tag.kind === "project" && (
        <div className="space-y-1.5">
          <div>
            <div className="text-[11px] font-medium text-muted-foreground">Sözleşme bütçesi</div>
            <div className="text-[11px] text-muted-foreground/80">
              Anlaşılan adam-gün; projeye bugüne kadar yazılan süreyle kıyaslanır, %80'de ve dolunca bildirilir. Bir
              adam-gün {String(dayHours).replace(".", ",")} saat (Zaman çizelgesi ayarı).
            </div>
          </div>
          <BudgetField value={tag.budgetDays} onSave={(d) => run(() => api.setProjectBudget(tag.id, d))()} />
          {budget && <BudgetMeter usage={budget} dayHours={dayHours} color={tagColor(tag)} className="max-w-xs" />}
        </div>
      )}
      {/* Arşivdeki projenin kuralları sınıflandırmaya girmez ve listede gelmez. */}
      {tag.archived ? (
        <p className="text-xs text-muted-foreground">
          Arşivdeki projenin kuralları çalışmaz; arşivden çıkarınca geri gelir.
        </p>
      ) : (
        (text.primary === "title" ? (["title", "domain", "app"] as const) : (["app", "domain", "title"] as const)).map(
          (field) => (
            <RuleList key={field} tag={tag} field={field} rules={rules} apps={apps} run={run} allTags={allTags} />
          ),
        )
      )}
      <div className="flex justify-end gap-1">
        {tag.kind === "project" && tag.archived && (
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            onClick={run(() => undoable(api.unarchiveProject(tag.id), `“${tag.name}” arşivden çıkarıldı`))}
          >
            <ArchiveRestore /> Arşivden çıkar
          </Button>
        )}
        {tag.kind === "project" && !tag.archived && (
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            title="Seçicilerden kalkar ve yeni süre toplamaz; geçmiş kayıtları ve toplamları korunur."
            onClick={run(() => undoable(api.archiveProject(tag.id), `“${tag.name}” arşivlendi`))}
          >
            <Archive /> Arşivle
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-muted-foreground hover:text-destructive"
          title={text.deleteHint}
          onClick={run(() => undoable(api.deleteTag(tag.id), `“${tag.name}” silindi`))}
        >
          <Trash2 /> {tag.kind === "project" ? "Projeyi sil" : "Kategoriyi sil"}
        </Button>
      </div>
    </div>
  );
}

function NameAndColor({ tag, run }: { tag: Tag; run: Run }) {
  const [name, setName] = useState(tag.name);
  // Başka cihazdan gelen yeniden adlandırma eski adla ezilmesin.
  useEffect(() => setName(tag.name), [tag.name]);
  return (
    <div className="space-y-1.5">
      <div className="text-[11px] font-medium text-muted-foreground">Ad ve renk</div>
      <div className="flex items-center gap-2">
        <Input
          className="h-8 max-w-xs text-sm"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== tag.name && run(() => api.saveTag({ ...tag, name: name.trim() }))()}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          aria-label="Ad"
        />
        <div className="flex gap-1" role="radiogroup" aria-label="Renk">
          {[1, 2, 3, 4, 5, 6, 7, 8].map((c) => (
            <button
              key={c}
              type="button"
              role="radio"
              aria-checked={c === tag.color}
              aria-label={`Renk ${c}`}
              className="grid size-6 place-items-center rounded-full text-white transition-transform hover:scale-110"
              style={{ background: `var(--c${c})` }}
              onClick={run(() => api.saveTag({ ...tag, color: c }))}
            >
              {c === tag.color && <Check className="size-3.5" />}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

const FIELD_TEXT: Record<RuleField, { title: string; hint: string; none: string }> = {
  title: {
    title: "Pencere başlığında geçen sözcükler",
    hint: "Büyük/küçük harf fark etmez; sözcüğün başlıkta bir yerde geçmesi yeter.",
    none: "Sözcük yok",
  },
  app: {
    title: "Uygulamalar",
    hint: "Bu uygulamalarda geçen süre buraya yazılır (başlık kuralı başka yere götürmedikçe).",
    none: "Uygulama yok",
  },
  domain: {
    title: "Web siteleri",
    hint: "Tarayıcıda bu adreslerde geçen süre. “togg.com” alt alan adlarını da kapsar, “github.com/firma” yalnızca o yolun altını.",
    none: "Site yok",
  },
};

/** Sözcük ve site kuralları yazılarak eklenir; uygulamalar listeden seçilir. */
const TYPED: Partial<Record<RuleField, { placeholder: string; label: string }>> = {
  title: { placeholder: "Sözcük ekle…", label: "Başlıkta aranacak sözcük" },
  domain: { placeholder: "örn. jira.togg.com", label: "Web sitesi adresi" },
};

/** Bir türdeki kurallar (sözcükler ya da uygulamalar) ve ekleme. */
function RuleList({
  tag,
  field,
  rules,
  apps,
  run,
  allTags,
}: {
  tag: Tag;
  field: RuleField;
  rules: Rule[];
  apps: UsageTotal[];
  run: Run;
  allTags: Tag[];
}) {
  const [word, setWord] = useState("");
  const appNames = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);
  const mine = rules.filter((r) => r.field === field);
  const shown = mine.filter((r) => !foreignRule(r));
  const hidden = mine.filter(foreignRule);
  const t = FIELD_TEXT[field];
  const typed = TYPED[field];
  return (
    <div className="space-y-1.5">
      <div>
        <div className="text-[11px] font-medium text-muted-foreground">{t.title}</div>
        <div className="text-[11px] text-muted-foreground/80">{t.hint}</div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {shown.map((r) => (
          <span
            key={r.id}
            className="inline-flex h-7 items-center gap-1 rounded-md border bg-card pr-0.5 pl-2 text-xs"
            title={r.pattern}
          >
            <span className="max-w-56 truncate">
              {field === "app" ? (appNames.get(r.pattern) ?? r.pattern) : r.pattern}
            </span>
            <button
              type="button"
              className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
              onClick={run(() =>
                undoable(
                  api.deleteRule(r.id),
                  `“${field === "app" ? (appNames.get(r.pattern) ?? r.pattern) : r.pattern}” kuralı kaldırıldı`,
                ),
              )}
              aria-label={`${r.pattern} kuralını sil`}
            >
              <X className="size-3" />
            </button>
          </span>
        ))}
        {shown.length === 0 && <span className="text-xs text-muted-foreground">{t.none}</span>}
        {hidden.length > 0 && (
          <span className="text-[11px] text-muted-foreground" title={hidden.map((r) => r.pattern).join("\n")}>
            +{hidden.length} başka işletim sistemi için
          </span>
        )}
        {typed ? (
          <form
            className="flex items-center gap-1"
            onSubmit={(e) => {
              e.preventDefault();
              if (!word.trim()) return;
              run(async () => {
                await api.addRule(tag.id, field, word.trim());
                setWord("");
              })();
            }}
          >
            <Input
              className="h-7 w-40 text-xs"
              value={word}
              onChange={(e) => setWord(e.target.value)}
              placeholder={typed.placeholder}
              aria-label={typed.label}
            />
            {word.trim() && (
              <Button type="submit" size="sm" variant="outline" className="h-7">
                Ekle
              </Button>
            )}
          </form>
        ) : (
          <AppPicker
            apps={apps.filter((a) => !mine.some((r) => r.pattern === a.key))}
            onPick={(key) => run(() => api.addRule(tag.id, "app", key))()}
          />
        )}
      </div>
      {typed && word.trim() && <RulePreview tagId={tag.id} field={field} pattern={word} tags={allTags} />}
    </div>
  );
}

/** Aranabilir uygulama listesi (en çok kullanılandan). */
function AppPicker({ apps, onPick }: { apps: UsageTotal[]; onPick: (key: string) => void }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const list = apps.filter((a) => a.label.toLocaleLowerCase("tr").includes(q.trim().toLocaleLowerCase("tr")));
  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (!o) setQ("");
      }}
    >
      <PopoverTrigger asChild>
        <Button type="button" size="sm" variant="outline" className="h-7">
          <Plus /> Uygulama ekle
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-1.5">
        <Input
          autoFocus
          className="mb-1 h-7 text-xs"
          placeholder="Uygulama ara"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          aria-label="Uygulama ara"
        />
        <ul className="max-h-64 overflow-y-auto">
          {list.length === 0 && <li className="px-2 py-1.5 text-xs text-muted-foreground">Uygulama bulunamadı</li>}
          {list.map((a) => (
            <li key={a.key}>
              <button
                type="button"
                className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs hover:bg-accent"
                onClick={() => {
                  setOpen(false);
                  setQ("");
                  onPick(a.key);
                }}
              >
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
                <span className="shrink-0 text-[11px] text-muted-foreground tabular">{formatDuration(a.seconds)}</span>
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}
