import { formatDuration, type RuleSuggestion } from "../api";

/** Önerilen kuralın kapsadığı süre: "LOY- ile başlayan iş anahtarları", "github.com/firma sayfaları"… */
export function ruleSubject(s: Pick<RuleSuggestion, "field" | "pattern" | "source">): string {
  if (s.source === "issueKey") return `${s.pattern} ile başlayan iş anahtarları`;
  if (s.field === "domain") return s.pattern.includes("/") ? `${s.pattern} altındaki sayfalar` : `${s.pattern} sitesi`;
  if (s.field === "app") return `${s.pattern} uygulaması`;
  return `Başlığında “${s.pattern}” geçen pencereler`;
}

/** Kartın ve bildirimin cümlesi: "LOY- ile başlayan iş anahtarları → Sadakat". */
export function ruleSentence(s: Pick<RuleSuggestion, "field" | "pattern" | "source" | "projectName">): string {
  return `${ruleSubject(s)} → ${s.projectName}`;
}

/** Neden önerildiği: "Son 30 günde 6 kez elle atandı (4 günde), 4sa 20dk". */
export function ruleReason(s: Pick<RuleSuggestion, "assignments" | "days" | "manualSeconds">): string {
  const days = s.days > 1 ? ` (${s.days} günde)` : "";
  return `Son 30 günde ${s.assignments} kez elle atandı${days}, ${formatDuration(s.manualSeconds)}`;
}
