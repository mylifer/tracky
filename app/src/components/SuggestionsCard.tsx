import { useState } from "react";
import { ArrowRight, FolderGit2, Sparkles, X } from "lucide-react";
import { api, formatDuration, type Suggestions, type Tag } from "../api";
import { tagColor } from "../lib/tags";
import { Button } from "./ui/button";
import { friendlyError } from "../lib/feedback";

/**
 * Otomatik öneriler: başlıklardan bulunan projeler, tanınan uygulama ve siteler için
 * kategoriler. Ekle/Ata bir etiket ya da kural oluşturur; çarpı öneriyi bir daha göstermez.
 */
export function SuggestionsCard({
  suggestions,
  tags,
  onChanged,
  onError,
}: {
  suggestions: Suggestions;
  tags: Tag[];
  onChanged: () => void;
  onError: (e: string) => void;
}) {
  // Aynı satıra art arda tıklanmasın.
  const [busy, setBusy] = useState<string | null>(null);
  const { projects, categories } = suggestions;
  if (projects.length === 0 && categories.length === 0) return null;

  const run = (key: string, f: () => Promise<unknown>) => async () => {
    setBusy(key);
    try {
      await f();
      onChanged();
    } catch (e) {
      onError(friendlyError(e));
    } finally {
      setBusy(null);
    }
  };
  const dismiss = (key: string) => (
    <Button
      size="icon-sm"
      variant="ghost"
      className="text-muted-foreground"
      aria-label="Yoksay"
      title="Yoksay: bir daha önerme"
      disabled={busy === key}
      onClick={run(key, () => api.dismissSuggestion(key))}
    >
      <X />
    </Button>
  );

  return (
    <section className="space-y-2">
      <div className="px-1">
        <h2 className="flex items-center gap-1.5 text-[13px] font-semibold">
          <Sparkles className="size-3.5 text-primary" />
          Öneriler
        </h2>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Son iki haftanın pencere başlıklarından ve tanınan uygulamalardan bulundu. Veriler bilgisayardan çıkmaz.
        </p>
      </div>
      <ul className="divide-y rounded-xl border bg-card shadow-xs">
        {projects.map((p) => (
          <li key={p.key} className="flex items-center gap-3 px-4 py-2.5">
            <FolderGit2 className="size-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px]">
                <span className="text-muted-foreground">Proje: </span>
                <span className="font-medium">{p.name}</span>
              </div>
              <div className="truncate text-xs text-muted-foreground">
                {formatDuration(p.seconds)} · {p.apps.join(", ")}
              </div>
            </div>
            <Button size="sm" disabled={busy === p.key} onClick={run(p.key, () => api.acceptProject(p.name))}>
              Proje ekle
            </Button>
            {dismiss(p.key)}
          </li>
        ))}
        {categories.map((c) => {
          const tag = tags.find((t) => t.id === c.categoryId);
          return (
            <li key={c.key} className="flex items-center gap-3 px-4 py-2.5">
              <i className="mx-1 size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-1.5 text-[13px]">
                  <span className="truncate font-medium">{c.label}</span>
                  <ArrowRight className="size-3 shrink-0 text-muted-foreground" />
                  <span className="truncate">{tag?.name}</span>
                </div>
                <div className="truncate text-xs text-muted-foreground">
                  {formatDuration(c.seconds)} kategorisiz ·{" "}
                  {c.field === "app" ? "uygulama" : `başlığında "${c.pattern}" geçen pencereler`}
                </div>
              </div>
              <Button
                size="sm"
                variant="outline"
                disabled={busy === c.key}
                onClick={run(c.key, () => api.acceptCategory(c))}
              >
                Ata
              </Button>
              {dismiss(c.key)}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
