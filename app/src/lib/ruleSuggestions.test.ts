import { describe, expect, it } from "vitest";
import { ruleReason, ruleSentence, ruleSubject } from "./ruleSuggestions";

describe("kural önerisi metinleri", () => {
  it("desenin türüne göre ne kapsadığını söyler", () => {
    expect(ruleSubject({ field: "title", pattern: "LOY-", source: "issueKey" })).toBe(
      "LOY- ile başlayan iş anahtarları",
    );
    expect(ruleSubject({ field: "title", pattern: "kum", source: "titleWord" })).toBe(
      "Başlığında “kum” geçen pencereler",
    );
    expect(ruleSubject({ field: "domain", pattern: "jira.firma.com", source: "site" })).toBe("jira.firma.com sitesi");
    expect(ruleSubject({ field: "domain", pattern: "github.com/firma/kum", source: "site" })).toBe(
      "github.com/firma/kum altındaki sayfalar",
    );
  });

  it("projeyle cümle kurar", () => {
    expect(ruleSentence({ field: "title", pattern: "LOY-", source: "issueKey", projectName: "Sadakat" })).toBe(
      "LOY- ile başlayan iş anahtarları → Sadakat",
    );
  });

  it("gerekçede atama sayısını, günleri ve süreyi verir", () => {
    expect(ruleReason({ assignments: 6, days: 4, manualSeconds: 4 * 3600 + 20 * 60 })).toBe(
      "Son 30 günde 6 kez elle atandı (4 günde), 4sa 20dk",
    );
    expect(ruleReason({ assignments: 3, days: 1, manualSeconds: 45 * 60 })).toBe(
      "Son 30 günde 3 kez elle atandı, 45dk",
    );
  });
});
