import { useEffect, useState } from "react";
import { Monitor } from "lucide-react";
import { api } from "../api";
import { cn } from "../lib/utils";

/**
 * Mac'lerin kendi görselleri ("Bu Mac Hakkında"), cihaz kimliğine göre. Rapor ve takvim
 * verisi görseli taşımaz (her blokta tekrarlanmasın); liste bir kez alınır.
 */
let icons: Promise<Map<string, string>> | null = null;
const loaded = new Map<string, string>();

function loadIcons() {
  icons ??= api.listDevices().then(
    (devices) => {
      for (const d of devices) if (d.icon) loaded.set(d.id, d.icon);
      return loaded;
    },
    () => {
      icons = null;
      return loaded;
    },
  );
  return icons;
}

function useDeviceImage(id: string | undefined) {
  const [src, setSrc] = useState(() => (id ? loaded.get(id) : undefined));
  useEffect(() => {
    if (!id) return;
    let live = true;
    void loadIcons().then((m) => live && setSrc(m.get(id)));
    return () => {
      live = false;
    };
  }, [id]);
  return src;
}

/**
 * Apple'ın Mac sayfasındaki ürün menüsü ikonları (apple.com/mac, chapternav), boşlukları
 * kırpılmış kare çerçevede. Renk `currentColor`: açık ve koyu temaya kendiliğinden uyar.
 */
const SHAPES = {
  macStudio: {
    viewBox: "0 31 28 28",
    d: "m0 38v11.9199c0 .5966.4836 1.0801 1.0801 1.0801h.9199l1.5 1h21l1.5-1h.9199c.5966 0 1.0801-.4836 1.0801-1.0801v-11.9199zm23.875 12c-.6213 0-1.125-.5037-1.125-1.125s.5037-1.125 1.125-1.125 1.125.5037 1.125 1.125-.5037 1.125-1.125 1.125z",
  },
  macbookPro: {
    viewBox: "0 0 72 72",
    d: "m68 49v-23c0-1.1046-.8954-2-2-2h-34c-1.1046 0-2 .8954-2 2v23h-4v1.2122c0 .7416.6012 1.2878 1.3429 1.2878h2.3754l.2144.5h2.5713l.2144-.5h32.2817l.2144.5h2.5713l.2144-.5h2.6571c.7417 0 1.3429-.5461 1.3429-1.2878v-1.2122h-4zm-1.5 0h-35v-23c0-.2757.2242-.5.5-.5h14.5v.06c0 .5192.4209.94.9399.94h3.1201c.519 0 .9399-.4208.9399-.94v-.06h14.5c.2758 0 .5.2243.5.5v23zm-41.5 0h-19.5v-27c0-.2757.2242-.5.5-.5h17.5v.06c0 .5192.4209.94.9399.94h3.1201c.519 0 .9399-.4208.9399-.94v-.06h17.5c.2758 0 .5.2243.5.5v1.0184h1.5v-1.0184c0-1.1046-.8954-2-2-2h-39.9999c-1.1046 0-2 .8954-2 2v27h-4v1.2122c0 .7416.6797 1.2878 1.5181 1.2878h1.8298l.2144.5h2.5713l.2144-.5h19.0471c-.2502-.3646-.395-.8066-.395-1.2878v-1.2122z",
  },
  macbookAir: {
    viewBox: "0 1 72 72",
    d: "m68 50v-22.0193c0-1.0939-.8867-1.9807-1.9807-1.9807h-32.0386c-1.094 0-1.9807.8868-1.9807 1.9807v22.0193h-4v.5c0 .2761.1118.5261.293.7071.1809.181.4309.2929.707.2929h2l.2859.5h3.4282l.2859-.5h30l.2859.5h3.4282l.2859-.5h2c.5522 0 1-.4478 1-1v-.5zm-1.5 0h-33v-22.0193c0-.265.2156-.4807.4807-.4807h13.7693c0 .5522.4478 1 1 1h2.5c.5522 0 1-.4478 1-1h13.7693c.2651 0 .4807.2156.4807.4807zm-39.5 0h-20.5v-26c0-.2757.2244-.5.5-.5h16.7598c.0269.5552.4771 1 1.0391 1h3.4023c.562 0 1.0122-.4448 1.0391-1h16.7598c.2756 0 .5.2243.5.5v1h1.5v-1c0-1.1046-.8955-2-2-2h-39.0001c-1.1045 0-2 .8954-2 2v26h-5v.5c0 .5522.4478 1 1 1h3l.2859.5h3.4282l.2859-.5h19.2783c-.1724-.2953-.2783-.6341-.2783-1z",
  },
  imac: {
    viewBox: "0 6 50 50",
    d: "M19 52h12v-7.5H19V52zm29.5-42h-47A1.5 1.5 0 0 0 0 11.5v31A1.5 1.5 0 0 0 1.5 44h47a1.5 1.5 0 0 0 1.5-1.5v-31a1.5 1.5 0 0 0-1.5-1.5zm0 28h-47V12a.5.5 0 0 1 .5-.5h46a.5.5 0 0 1 .5.5v26z",
  },
  macMini: {
    viewBox: "0 39.5 18 18",
    d: "m0 45v5.4116c0 .325.2634.5884.5884.5884h.4116l1 1h14l1-1h.4116c.325 0 .5884-.2634.5884-.5884v-5.4116zm15.625 5.5c-.3452 0-.625-.2798-.625-.625s.2798-.625.625-.625.625.2798.625.625-.2798.625-.625.625z",
  },
  macPro: {
    viewBox: "-3 4 48 48",
    d: "m42 49v-40h-2v-4c0-.5523-.4478-1-1-1s-1 .4477-1 1v4h-34v-4c0-.5523-.4477-1-1-1s-1 .4477-1 1v4h-2v40h2v1.4592c0 .2761-.2239.5-.5.5h-1.5v1.0408h6v-1.0408h-1.5c-.2761 0-.5-.2239-.5-.5v-1.4592h34v1.4592c0 .2761-.2239.5-.5.5h-1.5v1.0408h6v-1.0408h-1.5c-.2761 0-.5-.2239-.5-.5v-1.4592zm-20.8566-25.6246c.1108-.2449.2487-.4725.4344-.6773.1739-.1912.3843-.3596.6252-.4897.2393-.1281.4916-.1931.7479-.2084.0118.0364.0157.0689.0157.0995v.1069c0 .2527-.0557.5089-.1591.7654-.105.2543-.2389.4819-.4167.6808-.1743.2045-.3694.3635-.5911.4818-.2183.1207-.4419.1779-.6696.1779-.0936 0-.1414-.0133-.1473-.0533-.002-.027-.002-.0748-.002-.1454 0-.2468.0556-.4896.1626-.7384zm3.5617 6.7043c-.1187.2335-.239.4572-.3635.6522-.1321.201-.2582.3808-.3749.532-.1281.1508-.2296.2581-.2965.3326-.1375.1147-.2734.2143-.4188.2871-.1359.0744-.2812.1031-.434.1031-.0975 0-.2162-.0208-.3522-.0612-.1414-.0419-.2812-.0862-.4266-.134-.1473-.0478-.2946-.0897-.4399-.1375-.1528-.0384-.2809-.0576-.4016-.0576-.1301 0-.2754.0192-.4247.067-.1567.0424-.3099.0901-.4611.1399-.1567.0513-.302.0936-.4286.1375-.1281.0384-.2448.0591-.3231.0591-.1262 0-.2371-.0208-.3463-.0591-.1129-.0439-.2292-.1148-.3518-.2104-.1222-.0975-.2429-.2276-.3768-.3921-.1395-.1626-.2888-.3616-.4533-.6064-.1551-.2256-.2871-.4744-.4207-.7419-.1226-.2738-.2394-.5511-.3271-.8458-.0862-.2867-.1591-.5754-.2048-.872-.0498-.2946-.0823-.5778-.0823-.8571 0-.4282.0615-.8262.1876-1.1917.1261-.3611.304-.6828.5261-.9468.2236-.2715.4798-.4744.7745-.6232.2926-.1438.6158-.2182.962-.2182.2563 0 .5587.0744.907.2182.3423.1489.5793.2236.7172.2236.0478 0 .132-.0211.2504-.067.1245-.0439.3099-.1069.5413-.1951.2296-.0784.4055-.134.5414-.1646.1379-.0306.2774-.0478.4208-.0478.3866 0 .7443.0917 1.0961.2773.3444.1818.6217.4227.8226.7286-.3713.2241-.6428.4916-.8132.8055-.1626.3079-.2465.6788-.2465 1.098 0 .4399.1129.8395.3478 1.203.2296.3655.5547.6428.972.8262-.0823.2507-.1838.5014-.2985.7384z",
  },
};

/** Model adının başı → ikon; özelden genele (önce "MacBook Pro", sonra "MacBook"). */
const APPLE: [string, keyof typeof SHAPES][] = [
  ["macbook pro", "macbookPro"],
  ["macbook", "macbookAir"],
  ["imac", "imac"],
  ["mac mini", "macMini"],
  ["mac studio", "macStudio"],
  ["mac pro", "macPro"],
];

/** Windows 11 logosu (dört kare); Apple ikonlarıyla aynı ağırlıkta görünsün diye kenar boşluklu. */
const WINDOWS = { viewBox: "-18 -18 124 124", d: "M0 0h42v42H0zM46 0h42v42H46zM0 46h42v42H0zM46 46h42v42H46z" };

/**
 * Bilgisayarın ikonu: Mac'in kendi görseli ("Bu Mac Hakkında"); o Mac görselini henüz
 * eşitlemediyse modelin Apple ikonu, Windows'ta Windows logosu, diğerlerinde çizgi ikon.
 */
export function DeviceIcon({
  id,
  icon,
  os,
  model,
  className,
}: {
  id?: string;
  /** Biliniyorsa görsel; yoksa `id` ile cihaz listesinden bulunur. */
  icon?: string;
  os?: string;
  model?: string;
  className?: string;
}) {
  const looked = useDeviceImage(icon ? undefined : id);
  const image = icon || looked;
  if (image)
    return (
      <img src={image} alt="" title={model} draggable={false} className={cn("shrink-0 object-contain", className)} />
    );
  const m = model?.toLowerCase() ?? "";
  const key = m ? APPLE.find(([prefix]) => m.startsWith(prefix))?.[1] : undefined;
  const shape = key ? SHAPES[key] : os === "windows" ? WINDOWS : undefined;
  if (!shape) return <Monitor className={cn("shrink-0", className)} aria-hidden />;
  const { viewBox, d } = shape;
  return (
    <svg viewBox={viewBox} fill="currentColor" className={cn("shrink-0", className)} aria-hidden>
      <title>{model || "Windows"}</title>
      <path d={d} />
    </svg>
  );
}
