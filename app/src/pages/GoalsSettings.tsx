import { useEffect, useState } from "react";
import { api, type Goals } from "../api";
import { ErrorText, SettingRow, SettingsGroup, ToggleRow } from "../components/settings";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";

const BREAK_OPTIONS = [30, 45, 50, 60, 75, 90, 120];
const DEFAULT_BREAK = 60;

export default function GoalsSettings() {
  const [goals, setGoals] = useState<Goals | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.goals().then(setGoals, (e) => setError(String(e)));
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
          value={goals.dailyHours}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (v >= 0.5 && v <= 16) save({ ...goals, dailyHours: v });
          }}
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
      {error && (
        <div className="px-4 py-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
    </SettingsGroup>
  );
}
