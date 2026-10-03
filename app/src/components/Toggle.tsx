/** Açıklamalı aç/kapa satırı (ayarlar kartlarında). */
export default function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: () => void;
}) {
  return (
    <label className="setting">
      <div>
        <strong>{label}</strong>
        <p className="muted">{hint}</p>
      </div>
      <span className="switch">
        <input type="checkbox" checked={checked} onChange={onChange} />
        <span />
      </span>
    </label>
  );
}
