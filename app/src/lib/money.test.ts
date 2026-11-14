import { describe, expect, it } from "vitest";
import { formatUsd } from "./money";

describe("formatUsd", () => {
  it("Türkçe ondalıkla iki basamak yazar", () => {
    expect(formatUsd(0.022)).toBe("$0,02");
    expect(formatUsd(1234.5)).toBe("$1.234,50");
  });
  it("sentin altını ve sıfırı ayırır", () => {
    expect(formatUsd(0.004)).toBe("< $0,01");
    expect(formatUsd(0)).toBe("$0,00");
  });
});
