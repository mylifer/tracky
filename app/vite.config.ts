import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri sabit bir port bekler; dosya izleyici Rust tarafını yok saymalı.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
