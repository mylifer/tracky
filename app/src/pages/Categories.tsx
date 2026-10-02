import { useEffect, useMemo, useState } from "react";
import { api, type Rule, type RuleField, type Tag, type TagKind, type UsageTotal } from "../api";
import { nextColor, tagColor } from "../lib/tags";

const KIND_TITLE: Record<TagKind, string> = { category: "Kategoriler", project: "Projeler" };
const KIND_HINT: Record<TagKind, string> = {
  category: "Uygulamaları gruplar. Bir oturum tek kategoriye girer; başlık kuralları uygulama kurallarından önce gelir.",
  project: "Pencere başlığında geçen bir kelimeyle uygulamalar arası işleri toplar (örn. \"fintrack\").",
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
    <div className="page">
      <header>
        <h1>Kategoriler ve projeler</h1>
      </header>
      {error && <p className="error">{error}</p>}
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
    </div>
  );
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
    <section className="card">
      <h2>{KIND_TITLE[kind]}</h2>
      <p className="muted hint">{KIND_HINT[kind]}</p>
      <div className="tag-cards">
        {tags.map((t) => (
          <TagCard key={t.id} tag={t} rules={rules.filter((r) => r.tagId === t.id)} apps={apps} run={run} />
        ))}
      </div>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={kind === "category" ? "Yeni kategori adı" : "Yeni proje adı"}
        />
        <button type="submit" disabled={!name.trim()}>
          Ekle
        </button>
      </form>
    </section>
  );
}

function TagCard({ tag, rules, apps, run }: { tag: Tag; rules: Rule[]; apps: UsageTotal[]; run: Run }) {
  const [field, setField] = useState<RuleField>(tag.kind === "project" ? "title" : "app");
  const [pattern, setPattern] = useState("");
  const [name, setName] = useState(tag.name);
  const appNames = useMemo(() => new Map(apps.map((a) => [a.key, a.label])), [apps]);

  const addRule = run(async () => {
    if (!pattern.trim()) return;
    await api.addRule(tag.id, field, pattern);
    setPattern("");
  });

  return (
    <div className="tag-card">
      <div className="tag-card-head">
        <ColorPicker
          value={tag.color}
          onChange={(color) => run(() => api.saveTag({ ...tag, color }))()}
        />
        <input
          className="tag-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== tag.name && run(() => api.saveTag({ ...tag, name }))()}
          aria-label="Ad"
        />
        <button
          className="ghost small danger"
          onClick={() => {
            if (confirm(`"${tag.name}" silinsin mi? Kuralları da silinir; kayıtlar silinmez.`))
              run(() => api.deleteTag(tag.id))();
          }}
        >
          Sil
        </button>
      </div>
      <ul className="rules">
        {rules.map((r) => (
          <li key={r.id}>
            <span className="rule-kind">{r.field === "app" ? "Uygulama" : "Başlıkta"}</span>
            <span className="rule-pattern" title={r.pattern}>
              {r.field === "app" ? appNames.get(r.pattern) ?? r.pattern : `“${r.pattern}”`}
            </span>
            <button className="ghost icon small" onClick={run(() => api.deleteRule(r.id))} aria-label="Kuralı sil">
              ×
            </button>
          </li>
        ))}
        {rules.length === 0 && <li className="muted">Kural yok</li>}
      </ul>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          addRule();
        }}
      >
        <select value={field} onChange={(e) => setField(e.target.value as RuleField)} aria-label="Kural türü">
          <option value="app">Uygulama</option>
          <option value="title">Başlıkta geçen</option>
        </select>
        {field === "app" ? (
          <select value={pattern} onChange={(e) => setPattern(e.target.value)} aria-label="Uygulama">
            <option value="">Uygulama seç…</option>
            {apps.map((a) => (
              <option key={a.key} value={a.key}>
                {a.label}
              </option>
            ))}
          </select>
        ) : (
          <input value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder="örn. fintrack" />
        )}
        <button type="submit" disabled={!pattern.trim()}>
          Kural ekle
        </button>
      </form>
    </div>
  );
}

function ColorPicker({ value, onChange }: { value: number; onChange: (c: number) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="color-picker">
      <button
        className="swatch-btn"
        style={{ background: tagColor({ id: "", kind: "category", name: "", color: value }) }}
        onClick={() => setOpen(!open)}
        aria-label="Renk seç"
      />
      {open && (
        <span className="swatches">
          {[1, 2, 3, 4, 5, 6, 7, 8].map((c) => (
            <button
              key={c}
              className={`swatch-btn ${c === value ? "on" : ""}`}
              style={{ background: `var(--c${c})` }}
              onClick={() => {
                setOpen(false);
                onChange(c);
              }}
              aria-label={`Renk ${c}`}
            />
          ))}
        </span>
      )}
    </span>
  );
}
