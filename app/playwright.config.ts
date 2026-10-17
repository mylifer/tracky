import { defineConfig, devices } from "@playwright/test";

// Uçtan uca testler: arayüz sahte arka uçla (e2e/backend.ts) tarayıcıda açılır. Paketlenen
// uygulama macOS'ta WebKit (WKWebView), Windows'ta Chromium (WebView2) kullanır; CI ikisini de koşar.
const PORT = 1421;

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
    locale: "tr-TR",
    timezoneId: "Europe/Istanbul",
    viewport: { width: 1280, height: 820 },
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 820 } } },
    { name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 1280, height: 820 } } },
  ],
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}/e2e/harness.html`,
    reuseExistingServer: !process.env.CI,
  },
});
