import { useEffect } from "react";

const query = "(prefers-color-scheme: dark)";

export type ThemePref = "system" | "light" | "dark";

/** Görünüm tercihini `<html class="dark">` ile uygular; "system" sistemi izler. */
export function useTheme(pref: ThemePref) {
  useEffect(() => {
    const media = window.matchMedia(query);
    const apply = () =>
      document.documentElement.classList.toggle("dark", pref === "dark" || (pref === "system" && media.matches));
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [pref]);
}

/** Platform ve pencere malzemesi CSS'e `data-platform` / `data-effect` olarak geçer. */
export function applyPlatform(platform: string, effect: string) {
  const root = document.documentElement;
  root.dataset.platform = platform;
  root.dataset.effect = effect;
}
