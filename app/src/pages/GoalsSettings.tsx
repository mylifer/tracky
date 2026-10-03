import { useEffect, useState } from "react";
import { api, type Goals } from "../api";
import Toggle from "../components/Toggle";

const BREAK_OPTIONS = [30, 45, 50, 60, 75, 90, 120];
const DEFAULT_BREAK = 60;

export default function GoalsSettings() {
  const [goals, setGoals] = useState<Goals | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.goals().then(setGoals, (e) => setError(String(e)));
  }, []);

  if (!goals) return error ? <p className="error">{error}</p> : null;

  function save(next: Goals) {
    setGoals(next);
    api.saveGoals(next).catch((e) => setError(String(e)));
  }

  const breakOn = goals.breakAfterMinutes !== null;

  return (
    <section className="card settings">
      <h2>Hedefler ve hatırlatıcılar</h2>
      {error && <p className="error">{error}</p>}
      <div className="setting">
        <div>
          <strong>Günlük çalışma hedefi</strong>
          <p className="muted">Özet panelindeki "hedefin yüzdesi" buna göre hesaplanır.</p>
        </div>
        <span className="number-field">
          <input
            type="number"
            min={1}
            max={16}
            step={0.5}
            value={goals.dailyHours}
            onChange={(e) => {
              const v = Number(e.target.value);
              if (v >= 0.5 && v <= 16) save({ ...goals, dailyHours: v });
            }}
          />
          saat
        </span>
      </div>
      <Toggle
        label="Hedefe ulaşınca bildir"
        hint="Günün çalışma hedefi dolduğunda bir bildirim gösterilir."
        checked={goals.notifyGoal}
        onChange={() => save({ ...goals, notifyGoal: !goals.notifyGoal })}
      />
      <Toggle
        label="Mola hatırlatıcı"
        hint="Uzun süre ara vermeden çalışınca mola vermeni hatırlatır. 5 dakikalık bir ara sayacı sıfırlar."
        checked={breakOn}
        onChange={() => save({ ...goals, breakAfterMinutes: breakOn ? null : DEFAULT_BREAK })}
      />
      {breakOn && (
        <div className="setting">
          <div>
            <strong>Hatırlatma aralığı</strong>
            <p className="muted">Bu kadar kesintisiz çalışınca, sonra da aynı aralıklarla.</p>
          </div>
          <select
            value={goals.breakAfterMinutes ?? DEFAULT_BREAK}
            onChange={(e) => save({ ...goals, breakAfterMinutes: Number(e.target.value) })}
          >
            {BREAK_OPTIONS.map((m) => (
              <option key={m} value={m}>
                {m} dakika
              </option>
            ))}
          </select>
        </div>
      )}
    </section>
  );
}
