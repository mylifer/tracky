import type { ReactNode } from "react";

/** 0–100 odak skoru halkası (degrade çizgi, ortada sayı). */
export function ScoreRing({ score, size = 86 }: { score: number; size?: number }) {
  const r = (size - 12) / 2;
  const c = 2 * Math.PI * r;
  const filled = (Math.max(0, Math.min(100, score)) / 100) * c;
  return (
    <div className="ring" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        <defs>
          <linearGradient id="ring-grad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--accent-2)" />
            <stop offset="100%" stopColor="var(--accent)" />
          </linearGradient>
        </defs>
        <circle cx={size / 2} cy={size / 2} r={r} className="ring-track" strokeWidth={8} fill="none" />
        {score > 0 && <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          stroke="url(#ring-grad)"
          strokeWidth={8}
          fill="none"
          strokeLinecap="round"
          strokeDasharray={`${filled} ${c}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />}
      </svg>
      <span className="ring-value">{score}</span>
    </div>
  );
}

export function scoreLabel(score: number): string {
  if (score >= 80) return "Mükemmel odak";
  if (score >= 60) return "İyi odak";
  if (score >= 40) return "Orta";
  if (score > 0) return "Dağınık";
  return "Henüz veri yok";
}

export function StatCard({
  icon,
  label,
  value,
  sub,
  children,
}: {
  icon?: ReactNode;
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="card stat">
      <div className="stat-label">
        {icon}
        {label}
      </div>
      <div className="stat-value">{value}</div>
      {sub && <div className="stat-sub">{sub}</div>}
      {children}
    </section>
  );
}

/** İnce ilerleme çubuğu (tek değer; renk vurgu). */
export function Meter({ ratio }: { ratio: number }) {
  return (
    <div className="meter" role="presentation">
      <span style={{ width: `${Math.round(Math.max(0, Math.min(1, ratio)) * 100)}%` }} />
    </div>
  );
}
