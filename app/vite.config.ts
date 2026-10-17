/// <reference types="vitest/config" />
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";

// Tauri sabit bir port bekler; dosya izleyici Rust tarafını yok saymalı.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  // Yaz saati testleri makinenin saat diliminden bağımsız olsun.
  // Uçtan uca testler (e2e/) Playwright'la ayrıca koşar.
  test: { env: { TZ: "Europe/Berlin" }, exclude: [...configDefaults.exclude, "e2e/**"] },
});
