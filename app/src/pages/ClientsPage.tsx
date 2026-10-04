import { useCallback, useEffect, useState } from "react";
import { Building2, ChevronDown, ChevronRight, Plus, Trash2, X } from "lucide-react";
import { api, formatDuration, type Client, type Tag } from "../api";
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
import { addDays, isoDate, today } from "../lib/dates";
import { clientColor, tagColor } from "../lib/tags";
import { cn } from "../lib/utils";
import { friendlyError } from "../lib/feedback";

/**
 * Müşteriler: her müşterinin projeleri ve son 7 gündeki toplam süresi. Projeler buradan ya da
 * Projeler sayfasından müşteriye bağlanır; bir proje tek müşteriye aittir.
 */
export default function ClientsPage({ onOpenProjects }: { onOpenProjects: () => void }) {
  const [clients, setClients] = useState<Client[]>([]);
  const [projects, setProjects] = useState<Tag[]>([]);
  const [links, setLinks] = useState<Record<string, string>>({});
  const [usage, setUsage] = useState<Map<string, number>>(new Map());
  const [open, setOpen] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const t = await api.taxonomy();
    setClients(t.clients);
    setProjects(t.tags.filter((x) => x.kind === "project"));
    setLinks(t.projectClients);
    const r = await api.report(isoDate(addDays(today(), -6)), 7, false).catch(() => null);
    if (r) setUsage(new Map(r.projects.filter((b) => b.id).map((b) => [b.id!, b.seconds])));
  }, []);
  useEffect(() => {
    load().catch((e) => setError(friendlyError(e)));
  }, [load]);

  const run = (f: () => Promise<unknown>) => async () => {
    try {
      setError(null);
      await f();
      await load();
    } catch (e) {
      setError(friendlyError(e));
    }
  };

  const projectsOf = (id: string) => projects.filter((p) => links[p.id] === id);
  const unlinked = projects.filter((p) => !links[p.id]);

  return (
    <Page title="Müşteriler">
      <div className="space-y-2.5">
        <p className="px-1 text-[13px] text-muted-foreground">
          Projelerini çalıştığın müşterilere bağla: raporlarda müşteri bazında toplam süreyi görürsün. Bir proje tek
          müşteriye aittir.
        </p>
        <AddClient
          clients={clients}
          onAdded={async (id) => {
            await load();
            setOpen(id);
          }}
          onError={setError}
        />
      </div>
      <ErrorText>{error}</ErrorText>

      <section className="space-y-2">
        <div className="flex items-center gap-2 px-1">
          <h2 className="text-[13px] font-semibold">
            Müşteriler <span className="font-normal text-muted-foreground tabular">{clients.length}</span>
          </h2>
          <span className="ml-auto text-[11px] text-muted-foreground">Son 7 gün</span>
        </div>
        {clients.length === 0 ? (
          <div className="flex items-start gap-3 rounded-xl border border-dashed px-4 py-5 text-sm text-muted-foreground">
            <Building2 className="size-5 shrink-0" />
            <p>Henüz müşteri yok. Üstten bir ad yaz (örn. Togg), sonra projelerini bağla.</p>
          </div>
        ) : (
          <ul className="divide-y rounded-xl border bg-card shadow-xs">
            {clients.map((c) => (
              <ClientItem
                key={c.id}
                client={c}
                color={clientColor(clients, c.id)}
                projects={projectsOf(c.id)}
                allProjects={projects}
                links={links}
                clients={clients}
                seconds={projectsOf(c.id).reduce((s, p) => s + (usage.get(p.id) ?? 0), 0)}
                open={open === c.id}
                onToggle={() => setOpen(open === c.id ? null : c.id)}
                run={run}
              />
            ))}
          </ul>
        )}
        {clients.length > 0 && unlinked.length > 0 && (
          <p className="px-1 text-xs text-muted-foreground">
            {unlinked.length} proje henüz bir müşteriye bağlı değil:{" "}
            {unlinked
              .slice(0, 4)
              .map((p) => p.name)
              .join(", ")}
            {unlinked.length > 4 ? "…" : ""} ·{" "}
            <button className="underline underline-offset-2 hover:text-foreground" onClick={onOpenProjects}>
              Projeler'de ata
            </button>
          </p>
        )}
      </section>
    </Page>
  );
}

type Run = (f: () => Promise<unknown>) => () => Promise<void>;

function AddClient({
  clients,
  onAdded,
  onError,
}: {
  clients: Client[];
  onAdded: (id: string) => void;
  onError: (e: string) => void;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const exists = clients.some((c) => c.name.toLocaleLowerCase("tr") === name.trim().toLocaleLowerCase("tr"));
  return (
    <form
      className="space-y-1.5 rounded-xl border bg-card px-4 py-3 shadow-xs"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim() || exists) return;
        setBusy(true);
        try {
          const c = await api.saveClient(null, name.trim());
          setName("");
          onAdded(c.id);
        } catch (err) {
          onError(String(err));
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
          placeholder="Yeni müşteri adı"
          aria-label="Yeni müşteri adı"
        />
        <Button type="submit" size="sm" disabled={!name.trim() || exists || busy}>
          <Plus /> Ekle
        </Button>
      </div>
      {exists && <p className="text-[11px] text-destructive">“{name.trim()}” zaten var.</p>}
    </form>
  );
}

function ClientItem({
  client,
  color,
  projects,
  allProjects,
  links,
  clients,
  seconds,
  open,
  onToggle,
  run,
}: {
  client: Client;
  color: string;
  projects: Tag[];
  allProjects: Tag[];
  links: Record<string, string>;
  clients: Client[];
  seconds: number;
  open: boolean;
  onToggle: () => void;
  run: Run;
}) {
  const [name, setName] = useState(client.name);
  useEffect(() => setName(client.name), [client.name]);
  const others = allProjects.filter((p) => links[p.id] !== client.id);
  const clientName = (id: string | undefined) => clients.find((c) => c.id === id)?.name;
  return (
    <li>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-accent/50"
      >
        <ChevronRight
          className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
        />
        <i className="size-2.5 shrink-0 rounded-full" style={{ background: color }} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium">{client.name}</span>
          <span className="block truncate text-[11px] text-muted-foreground">
            {projects.length ? projects.map((p) => p.name).join(", ") : "Proje yok"}
          </span>
        </span>
        <span className="shrink-0 text-xs text-muted-foreground tabular">
          {seconds ? formatDuration(seconds) : "—"}
        </span>
      </button>
      {open && (
        <div className="space-y-4 border-t bg-muted/20 px-4 py-3.5 pl-11">
          <div className="space-y-1.5">
            <div className="text-[11px] font-medium text-muted-foreground">Ad</div>
            <Input
              className="h-8 max-w-xs text-sm"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => name.trim() && name !== client.name && run(() => api.saveClient(client.id, name.trim()))()}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              aria-label="Müşteri adı"
            />
          </div>
          <div className="space-y-1.5">
            <div className="text-[11px] font-medium text-muted-foreground">Projeleri</div>
            <div className="flex flex-wrap items-center gap-1.5">
              {projects.map((p) => (
                <span
                  key={p.id}
                  className="inline-flex h-7 items-center gap-1.5 rounded-md border bg-card pr-0.5 pl-2 text-xs"
                >
                  <i className="size-2 rounded-full" style={{ background: tagColor(p) }} />
                  <span className="max-w-48 truncate">{p.name}</span>
                  <button
                    type="button"
                    className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
                    onClick={run(() => api.setProjectClient(p.id, null))}
                    aria-label={`${p.name} projesini bu müşteriden çıkar`}
                    title="Müşteriden çıkar (proje silinmez)"
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
              {projects.length === 0 && <span className="text-xs text-muted-foreground">Proje yok</span>}
              {others.length > 0 && (
                <span className="relative inline-flex items-center">
                  <select
                    value=""
                    onChange={(e) => e.target.value && run(() => api.setProjectClient(e.target.value, client.id))()}
                    className="h-7 appearance-none rounded-md border bg-transparent pr-7 pl-2 text-xs hover:bg-accent dark:bg-input/30"
                    aria-label="Proje bağla"
                  >
                    <option value="" disabled>
                      + Proje bağla…
                    </option>
                    {others.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                        {links[p.id] ? ` (şu an: ${clientName(links[p.id])})` : ""}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-muted-foreground" />
                </span>
              )}
            </div>
          </div>
          <div className="flex justify-end">
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="ghost" size="sm" className="text-muted-foreground hover:text-destructive">
                  <Trash2 /> Müşteriyi sil
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>“{client.name}” silinsin mi?</AlertDialogTitle>
                  <AlertDialogDescription>
                    Projeleri ve kayıtları silinmez; projeler müşterisiz kalır.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Vazgeç</AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-white hover:bg-destructive/90"
                    onClick={run(() => api.deleteClient(client.id))}
                  >
                    Sil
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </div>
      )}
    </li>
  );
}
