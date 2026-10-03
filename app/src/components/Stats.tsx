/** 0–100 odak skoru halkası (ortada sayı). */
export function ScoreRing({ score, size = 86 }: { score: number; size?: number }) {
  const stroke = Math.max(4, Math.round(size / 9));
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const filled = (Math.max(0, Math.min(100, score)) / 100) * c;
  return (
    <div className="relative shrink-0" style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        <circle cx={size / 2} cy={size / 2} r={r} stroke="var(--muted)" strokeWidth={stroke} fill="none" />
        {score > 0 && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            stroke="var(--focus)"
            strokeWidth={stroke}
            fill="none"
            strokeLinecap="round"
            strokeDasharray={`${filled} ${c}`}
            transform={`rotate(-90 ${size / 2} ${size / 2})`}
          />
        )}
      </svg>
      <span className="absolute inset-0 grid place-items-center text-[13px] font-semibold tabular">{score}</span>
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
