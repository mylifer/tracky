// Uygulamayı sahte arka uçla açar (yalnızca uçtan uca testlerde; paketlenen uygulamaya girmez).
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { createBackend, type FakeBackend } from "./backend";

declare global {
  interface Window {
    __kum: FakeBackend;
  }
}

const backend = createBackend();
window.__kum = backend;
mockWindows("main");
mockIPC((cmd, args) => backend.handle(cmd, args as Record<string, unknown>), { shouldMockEvents: true });
await import("../src/main");
