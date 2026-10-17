import { describe, expect, it } from "vitest";
import { latestGate } from "./latest";

describe("en son istek", () => {
  it("yalnızca en son açılan istek geçerlidir", () => {
    const begin = latestGate();
    const first = begin();
    expect(first()).toBe(true);
    const second = begin();
    // İlk isteğin yanıtı sonra gelse de işlenmez.
    expect(first()).toBe(false);
    expect(second()).toBe(true);
  });

  it("her bileşenin kapısı ayrıdır", () => {
    const a = latestGate();
    const b = latestGate();
    const fromA = a();
    b();
    expect(fromA()).toBe(true);
  });
});
