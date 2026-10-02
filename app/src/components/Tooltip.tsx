import { useState, type ReactNode } from "react";

export type TipState = { x: number; y: number; content: ReactNode } | null;

/** Fareyi izleyen bilgi kutusu; grafiğin kendi kapsayıcısına göre konumlanır. */
export function useTooltip() {
  const [tip, setTip] = useState<TipState>(null);
  const show = (e: React.MouseEvent, content: ReactNode) => {
    const box = (e.currentTarget as Element).closest(".chart")!.getBoundingClientRect();
    setTip({ x: e.clientX - box.left, y: e.clientY - box.top, content });
  };
  const hide = () => setTip(null);
  return { tip, show, hide };
}

export function Tooltip({ tip }: { tip: TipState }) {
  if (!tip) return null;
  return (
    <div className="tooltip" style={{ left: tip.x, top: tip.y }} role="tooltip">
      {tip.content}
    </div>
  );
}
