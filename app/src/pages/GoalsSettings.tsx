import { useEffect, useState } from "react";
import { Plus, X } from "lucide-react";
import { api, type Goals, type Tag } from "../api";
import { CategorySelect } from "../components/CategorySelect";
import { ErrorText, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Input } from "../components/ui/input";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { tagColor } from "../lib/tags";

const BREAK_OPTIONS = [30, 45, 50, 60, 75, 90, 120];
const DEFAULT_BREAK = 60;
const LIMIT_OPTIONS = [15, 30, 45, 60, 90, 120, 180, 240];

function minutesLabel(m: number) {
  return m < 60 ? `${m} dk` : m % 60 === 0 ? `${m / 60} sa` : `${Math.floor(m / 60)} sa ${m % 60} dk`;
}

export default function GoalsSettings() {
  const [goals, setGoals] = useState<Goals | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hours, setHours] = useState("");
  const [categories, setCategories] = useState<Tag[]>([]);

  useEffect(() => {
    api.goals().then(
      (g) => {
        setGoals(g);
        setHours(String(g.dailyHours));
      },
      (e) => setError(String(e)),
    );
    api.taxonomy().then((t) => setCategories(t.tags.filter((x) => x.kind === "category")));
  }, []);

  if (!goals) return <ErrorText>{error}</ErrorText>;

  function save(next: Goals) {
    setGoals(next);
    api.saveGoals(next).catch((e) => setError(String(e)));
  }

  const breakOn = goals.breakAfterMinutes !== null;

  return (
    <SettingsGroup title="Hedefler ve hatırlatıcılar">
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
      <LimitsBlock goals={goals} categories={categories} onChange={save} />
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
