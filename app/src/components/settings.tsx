import type { ReactNode } from "react";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

/** Sistem Ayarları tarzı grup: başlık ve altında ayrı çizgili satırlar. */
export function SettingsGroup({
  title,
  description,
  children,
  className,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("space-y-2", className)}>
      <div className="px-1">
        <h2 className="text-[13px] font-semibold">{title}</h2>
        {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
      </div>
      <div className="divide-y rounded-xl border bg-card shadow-xs">{children}</div>
    </section>
  );
}

/** Tek ayar satırı: solda ad ve açıklama, sağda denetim. */
export function SettingRow({
  label,
  hint,
  children,
  className,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex min-h-12 items-center justify-between gap-6 px-4 py-2.5", className)}>
      <div className="min-w-0">
        <div className="text-[13px] font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

/** Altında içerik olan (dikey) ayar satırı. */
export function SettingBlock({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="space-y-2.5 px-4 py-3">
      <div>
        <div className="text-[13px] font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>}
      </div>
      {children}
    </div>
  );
}

export function ToggleRow({
  label,
  hint,
  checked,
  onChange,
}: {
  label: ReactNode;
  hint?: ReactNode;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <SettingRow label={label} hint={hint}>
      <Switch checked={checked} onCheckedChange={onChange} />
    </SettingRow>
  );
}

/** Sayfa düzeni: başlık ve dar, ortalanmış içerik (ayarlar ve kategoriler). */
export function Page({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-3xl space-y-7 px-6 pt-2 pb-10">
      <h1 className="sr-only">{title}</h1>
      {children}
    </div>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  if (!children) return null;
  return <p className="px-1 text-xs text-destructive selectable">{children}</p>;
}
