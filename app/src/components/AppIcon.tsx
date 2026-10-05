import { useEffect, useState, type ReactNode } from "react";
import { Globe, PenLine } from "lucide-react";
import { api } from "../api";
import { cn } from "../lib/utils";

type Kind = "app" | "site";

/** Bulunan simgeler (`null` = yok); yeniden çizimde titremesin diye eşzamanlı okunur. */
const resolved = new Map<string, string | null>();
const pending = new Map<string, Promise<string | null>>();

/** Site simgeleri ağdan gelir: haftalık takvim onlarca alan adını aynı anda istemesin. */
const SITE_PARALLEL = 4;
let siteActive = 0;
const siteQueue: (() => void)[] = [];

async function siteSlot<T>(f: () => Promise<T>): Promise<T> {
  if (siteActive >= SITE_PARALLEL) await new Promise<void>((r) => siteQueue.push(r));
  siteActive++;
  try {
    return await f();
  } finally {
    siteActive--;
    siteQueue.shift()?.();
  }
}

function load(kind: Kind, key: string): Promise<string | null> {
  const id = `${kind}\u0000${key}`;
  let p = pending.get(id);
  if (!p) {
    const call = kind === "app" ? () => api.appIcon(key) : () => siteSlot(() => api.siteIcon(key));
    p = call()
      .catch(() => null)
      .then((icon) => {
        resolved.set(id, icon);
        return icon;
      });
    pending.set(id, p);
  }
  return p;
}

function useIcon(kind: Kind, key: string | null | undefined): string | null | undefined {
  const id = key ? `${kind}\u0000${key}` : null;
  const [icon, setIcon] = useState(() => (id ? resolved.get(id) : null));
  useEffect(() => {
    if (!key || !id) return setIcon(null);
    if (resolved.has(id)) return setIcon(resolved.get(id));
    setIcon(undefined);
    let live = true;
    load(kind, key).then((v) => live && setIcon(v));
    return () => {
      live = false;
    };
  }, [kind, key, id]);
  return icon;
}

function initial(name: string): string {
  return ([...name.trim()][0] ?? "?").toLocaleUpperCase("tr-TR");
}

/**
 * Uygulamanın kendi simgesi (macOS/Windows'tan). Bulunamazsa (elle kayıt, başka cihazdan
 * eşitlenen uygulama) `fallback`, o da verilmezse baş harf rozeti.
 */
export function AppIcon({
  appId,
  name,
  size = 16,
  className,
  fallback,
}: {
  appId: string;
  name: string;
  size?: number;
  className?: string;
  fallback?: ReactNode;
}) {
  const manual = appId.startsWith("kum.manual");
  const icon = useIcon("app", manual ? null : appId);
  const box = { width: size, height: size };
  if (icon)
    return <img src={icon} alt="" aria-hidden style={box} className={cn("shrink-0", className)} draggable={false} />;
  // Yüklenirken boş yer: simge gelince yazı kaymasın.
  if (icon === undefined) return <span aria-hidden style={box} className={cn("shrink-0", className)} />;
  if (fallback !== undefined) return <>{fallback}</>;
  return (
    <span
      aria-hidden
      style={{ ...box, fontSize: Math.round(size * 0.6) }}
      className={cn(
        "grid shrink-0 place-items-center rounded-[22%] bg-muted font-semibold text-muted-foreground",
        className,
      )}
    >
      {manual ? <PenLine style={{ width: size * 0.65, height: size * 0.65 }} /> : initial(name)}
    </span>
  );
}

/** Sitenin simgesi (favicon); yoksa küre. */
export function SiteIcon({ domain, size = 12, className }: { domain: string; size?: number; className?: string }) {
  const icon = useIcon("site", domain);
  const box = { width: size, height: size };
  if (icon) {
    return (
      <img
        src={icon}
        alt=""
        aria-hidden
        style={box}
        className={cn("shrink-0 rounded-[3px] object-contain", className)}
        draggable={false}
      />
    );
  }
  return <Globe aria-hidden style={box} className={cn("shrink-0", icon === undefined && "opacity-40", className)} />;
}

/** Bloğun başlıca uygulamaları üst üste binen küçük simgeler olarak. */
export function AppIconStack({
  apps,
  size = 14,
  max = 3,
  className,
}: {
  apps: { appId: string; appName: string }[];
  size?: number;
  max?: number;
  className?: string;
}) {
  const shown = apps.slice(0, max);
  if (shown.length === 0) return null;
  return (
    <span className={cn("flex shrink-0 items-center", className)} aria-hidden>
      {shown.map((a, i) => (
        <AppIcon
          key={a.appId}
          appId={a.appId}
          name={a.appName}
          size={size}
          className={cn(i > 0 && "-ml-[5px]", "drop-shadow-[0_0_0.5px_rgba(0,0,0,0.35)]")}
        />
      ))}
    </span>
  );
}
