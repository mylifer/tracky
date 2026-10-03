import { useEffect } from "react";

const query = "(prefers-color-scheme: dark)";

/** Sistemin açık/koyu temasını `<html class="dark">` ile izler. */
export function useSystemTheme() {
  useEffect(() => {
    const media = window.matchMedia(query);
    const apply = () => document.documentElement.classList.toggle("dark", media.matches);
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, []);
}

/** Platform ve pencere malzemesi CSS'e `data-platform` / `data-effect` olarak geçer. */
export function applyPlatform(platform: string, effect: string) {
  const root = document.documentElement;
  root.dataset.platform = platform;
  root.dataset.effect = effect;
}
