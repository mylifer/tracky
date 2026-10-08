import { Monitor } from "lucide-react";
import { cn } from "../lib/utils";
import imac from "../assets/devices/imac.png";
import imacPro from "../assets/devices/imac-pro.png";
import macMini from "../assets/devices/mac-mini.png";
import macPro from "../assets/devices/mac-pro.png";
import macStudio from "../assets/devices/mac-studio.png";
import macbook from "../assets/devices/macbook.png";
import macbookAir from "../assets/devices/macbook-air.png";
import macbookPro from "../assets/devices/macbook-pro.png";

/** Model adının başı → Apple'ın cihaz ikonu; özelden genele (önce "MacBook Pro", sonra "MacBook"). */
const APPLE: [string, string][] = [
  ["macbook pro", macbookPro],
  ["macbook air", macbookAir],
  ["macbook", macbook],
  ["imac pro", imacPro],
  ["imac", imac],
  ["mac mini", macMini],
  ["mac studio", macStudio],
  ["mac pro", macPro],
];

/** Bilgisayarın ikonu: Mac'lerde modelin gerçek görseli, diğerlerinde çizgi ikon. */
export function DeviceIcon({ model, className }: { model?: string; className?: string }) {
  const m = model?.toLowerCase() ?? "";
  const src = m ? APPLE.find(([prefix]) => m.startsWith(prefix))?.[1] : undefined;
  if (src)
    return (
      <img src={src} alt="" title={model} draggable={false} className={cn("shrink-0 object-contain", className)} />
    );
  return <Monitor className={cn("shrink-0", className)} aria-hidden />;
}
