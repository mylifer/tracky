import { useEffect, useState } from "react";
import { Check, Plus, X } from "lucide-react";
import { api, type Goals, type Tag } from "../api";
import { CategorySelect } from "../components/CategorySelect";
import { ErrorText, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Input } from "../components/ui/input";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { tagColor } from "../lib/tags";
import { cn } from "../lib/utils";

const BREAK_OPTIONS = [30, 45, 50, 60, 75, 90, 120];
const DEFAULT_BREAK = 60;
const LIMIT_OPTIONS = [15, 30, 45, 60, 90, 120, 180, 240];
/** Proje hedefi seçenekleri (saat/hafta). */
const PROJECT_GOAL_HOURS = [2, 5, 10, 15, 20, 25, 30, 40];
/** Gün sonu özeti saatleri (16:00–23:00). */
const SUMMARY_OPTIONS = Array.from({ length: 8 }, (_, i) => (16 + i) * 60);
const DEFAULT_SUMMARY = 18 * 60;

function minutesLabel(m: number) {
  return m < 60 ? `${m} dk` : m % 60 === 0 ? `${m / 60} sa` : `${Math.floor(m / 60)} sa ${m % 60} dk`;
}

const clock = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;

export default function GoalsSettings() {
  const [goals, setGoals] = useState<Goals | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hours, setHours] = useState("");
  const [categories, setCategories] = useState<Tag[]>([]);
  const [projects, setProjects] = useState<Tag[]>([]);

  useEffect(() => {
    api.goals().then(
      (g) => {
        setGoals(g);
        setHours(String(g.dailyHours));
      },
      (e) => setError(String(e)),
    );
    api.taxonomy().then((t) => {
      setCategories(t.tags.filter((x) => x.kind === "category"));
      setProjects(t.tags.filter((x) => x.kind === "project"));
    });
  }, []);

  if (!goals) return <ErrorText>{error}</ErrorText>;

  function save(next: Goals) {
    setGoals(next);
    api.saveGoals(next).catch((e) => setError(String(e)));
  }

  const breakOn = goals.breakAfterMinutes !== null;
  const summaryOn = goals.daySummaryAt !== null;

  return (
    <SettingsGroup id="hedefler" title="Hedefler ve hatırlatıcılar">
      <SettingRow label="Günlük çalışma hedefi" hint="Özetteki hedef yüzdesi buna göre hesaplanır.">
        <Input
          type="number"
          min={1}
          max={16}
          step={0.5}
          className="h-7 w-16 text-right tabular"
          value={hours}
          onChange={(e) => setHours(e.target.value)}
          onBlur={() => {
            // Yazarken değil, alandan çıkınca kaydet; geçersizse eski değere dön.
            const v = Number(hours.replace(",", "."));
            if (v >= 0.5 && v <= 16) {
              if (v !== goals.dailyHours) save({ ...goals, dailyHours: v });
            } else setHours(String(goals.dailyHours));
          }}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        <span className="text-xs text-muted-foreground">saat</span>
      </SettingRow>
      <ToggleRow
        label="Hedefe ulaşınca bildir"
        hint="Günün çalışma hedefi dolduğunda bir bildirim gösterilir."
        checked={goals.notifyGoal}
        onChange={(v) => save({ ...goals, notifyGoal: v })}
      />
      <ToggleRow
        label="Mola hatırlatıcı"
        hint="Uzun süre ara vermeden çalışınca mola vermeni hatırlatır. 5 dakikalık bir ara sayacı sıfırlar."
        checked={breakOn}
        onChange={(v) => save({ ...goals, breakAfterMinutes: v ? DEFAULT_BREAK : null })}
      />
      {breakOn && (
        <SettingRow label="Hatırlatma aralığı" hint="Bu kadar kesintisiz çalışınca, sonra da aynı aralıklarla.">
          <Select
            value={String(goals.breakAfterMinutes ?? DEFAULT_BREAK)}
            onValueChange={(v) => save({ ...goals, breakAfterMinutes: Number(v) })}
          >
            <SelectTrigger size="sm" className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="end">
              {BREAK_OPTIONS.map((m) => (
                <SelectItem key={m} value={String(m)}>
                  {m} dakika
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
      )}
      <ToggleRow
        label="Gün sonu özeti"
        hint="Akşam günün çalışma ve odak süresini, hedefe göre durumunu bildirir."
        checked={summaryOn}
        onChange={(v) => save({ ...goals, daySummaryAt: v ? DEFAULT_SUMMARY : null })}
      />
      {summaryOn && (
        <SettingRow label="Özet saati" hint="O gün en az 15 dakika çalışıldıysa, günde bir kez.">
          <Select
            value={String(goals.daySummaryAt ?? DEFAULT_SUMMARY)}
            onValueChange={(v) => save({ ...goals, daySummaryAt: Number(v) })}
          >
            <SelectTrigger size="sm" className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent align="end">
              {SUMMARY_OPTIONS.map((m) => (
                <SelectItem key={m} value={String(m)}>
                  {clock(m)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
      )}
      <ToggleRow
        label="Haftalık özet"
        hint="Yeni haftada ilk çalışmaya başlayınca geçen haftanın süresini, değişimini ve en yoğun gününü bildirir."
        checked={goals.weeklySummary}
        onChange={(v) => save({ ...goals, weeklySummary: v })}
      />
      <ToggleRow
        label="Odak koruması"
        hint="Odak zamanlayıcısı sürerken dikkat dağıtıcı bir kategorideki uygulamaya geçince uyarır."
        checked={goals.focusGuard}
        onChange={(v) => save({ ...goals, focusGuard: v })}
      />
      {goals.focusGuard && (
        <div className="space-y-2 px-4 py-3">
          <div className="text-[13px]">Dikkat dağıtıcı kategoriler</div>
          <div className="flex flex-wrap gap-1.5">
            {categories.map((c) => {
              const on = goals.distracting.includes(c.id);
              return (
                <button
                  key={c.id}
                  aria-pressed={on}
                  onClick={() =>
                    save({
                      ...goals,
                      distracting: on ? goals.distracting.filter((id) => id !== c.id) : [...goals.distracting, c.id],
                    })
                  }
                  className={cn(
                    "flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors",
                    on ? "border-primary/40 bg-primary/12 font-medium" : "text-muted-foreground hover:bg-accent",
                  )}
                >
                  <i className="size-2 rounded-full" style={{ background: tagColor(c) }} />
                  {c.name}
                  {on && <Check className="size-3 text-primary" />}
                </button>
              );
            })}
          </div>
        </div>
      )}
      <LimitsBlock goals={goals} categories={categories} onChange={save} />
      <ProjectGoalsBlock goals={goals} projects={projects} onChange={save} />
      {error && (
        <div className="px-4 py-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
    </SettingsGroup>
  );
}

/** Kategori limitleri: her satırda kategori, günlük süre ve kaldırma. */
function LimitsBlock({
  goals,
  categories,
  onChange,
}: {
  goals: Goals;
  categories: Tag[];
  onChange: (g: Goals) => void;
}) {
  const limits = goals.limits.filter((l) => categories.some((c) => c.id === l.categoryId));
  const free = categories.filter((c) => !limits.some((l) => l.categoryId === c.id));
  const set = (next: Goals["limits"]) => onChange({ ...goals, limits: next });

  return (
    <div className="space-y-2.5 px-4 py-3">
      <div>
        <div className="text-[13px]">Kategori limitleri</div>
        <p className="text-xs text-muted-foreground">
          Bir kategoride günlük sınırın %80'ine gelince ve sınır dolunca bildirim gösterilir.
        </p>
      </div>
      {limits.length > 0 && (
        <ul className="space-y-1.5">
          {limits.map((l) => {
            const tag = categories.find((c) => c.id === l.categoryId);
            return (
              <li key={l.categoryId} className="flex items-center gap-2">
                <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
                <span className="min-w-0 flex-1 truncate">{tag?.name}</span>
                <span className="text-xs text-muted-foreground">günde en çok</span>
                <Select
                  value={String(l.minutes)}
                  onValueChange={(v) =>
                    set(limits.map((x) => (x.categoryId === l.categoryId ? { ...x, minutes: Number(v) } : x)))
                  }
                >
                  <SelectTrigger size="sm" className="w-28" aria-label={`${tag?.name} limiti`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    {LIMIT_OPTIONS.map((m) => (
                      <SelectItem key={m} value={String(m)}>
                        {minutesLabel(m)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-muted-foreground"
                  onClick={() => set(limits.filter((x) => x.categoryId !== l.categoryId))}
                  aria-label="Limiti kaldır"
                >
                  <X className="size-3.5" />
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      {free.length > 0 && (
        <CategorySelect
          value={null}
          onChange={(id) => id && set([...limits, { categoryId: id, minutes: 60 }])}
          categories={free}
          placeholder="Limit ekle…"
          icon={<Plus className="size-3.5" />}
          className="w-56"
        />
      )}
    </div>
  );
}

/** Proje hedefleri: her satırda proje, haftalık saat ve kaldırma. */
function ProjectGoalsBlock({
  goals,
  projects,
  onChange,
}: {
  goals: Goals;
  projects: Tag[];
  onChange: (g: Goals) => void;
}) {
  const rows = goals.projectGoals.filter((g) => projects.some((p) => p.id === g.projectId));
  const free = projects.filter((p) => !rows.some((g) => g.projectId === p.id));
  const set = (next: Goals["projectGoals"]) => onChange({ ...goals, projectGoals: next });

  return (
    <div className="space-y-2.5 px-4 py-3">
      <div>
        <div className="text-[13px]">Proje hedefleri</div>
        <p className="text-xs text-muted-foreground">
          {projects.length === 0
            ? "Önce Kategoriler sayfasından bir proje ekle."
            : "Hafta içinde hedef dolunca bildirim gösterilir; ilerleme Eğilimler ve hafta özetinde görünür."}
        </p>
      </div>
      {rows.length > 0 && (
        <ul className="space-y-1.5">
          {rows.map((g) => {
            const tag = projects.find((p) => p.id === g.projectId);
            return (
              <li key={g.projectId} className="flex items-center gap-2">
                <i className="size-2 shrink-0 rounded-full" style={{ background: tagColor(tag) }} />
                <span className="min-w-0 flex-1 truncate">{tag?.name}</span>
                <span className="text-xs text-muted-foreground">haftada</span>
                <Select
                  value={String(g.minutes)}
                  onValueChange={(v) =>
                    set(rows.map((x) => (x.projectId === g.projectId ? { ...x, minutes: Number(v) } : x)))
                  }
                >
                  <SelectTrigger size="sm" className="w-28" aria-label={`${tag?.name} hedefi`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent align="end">
                    {PROJECT_GOAL_HOURS.map((h) => (
                      <SelectItem key={h} value={String(h * 60)}>
                        {h} sa
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-muted-foreground"
                  onClick={() => set(rows.filter((x) => x.projectId !== g.projectId))}
                  aria-label="Hedefi kaldır"
                >
                  <X className="size-3.5" />
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      {free.length > 0 && (
        <CategorySelect
          value={null}
          onChange={(id) => id && set([...rows, { projectId: id, minutes: 10 * 60 }])}
          categories={free}
          placeholder="Hedef ekle…"
          icon={<Plus className="size-3.5" />}
          className="w-56"
        />
      )}
    </div>
  );
}
