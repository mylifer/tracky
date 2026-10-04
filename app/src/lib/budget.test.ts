import { describe, expect, it } from "vitest";
import { budgetState, burnDown, formatDays, weeksLeft } from "./budget";
import { activeProjects, archivedProjects } from "./tags";
import type { Tag } from "../api";

const DAY = 8 * 3600;

describe("sözleşme bütçesi", () => {
  it("adam-günü gün saatine göre yazar", () => {
    expect(formatDays(12 * 3600, 8)).toBe("1,5");
    expect(formatDays(15 * 3600, 7.5)).toBe("2");
    expect(formatDays(DAY, 0)).toBe("1");
  });

  it("%80'de yaklaştı, %100'de doldu", () => {
    const u = (used: number) => ({ id: "p", budgetDays: 10, budgetSeconds: 10 * DAY, usedSeconds: used });
    expect(budgetState(u(7 * DAY))).toBe("ok");
    expect(budgetState(u(8 * DAY))).toBe("near");
    expect(budgetState(u(10 * DAY))).toBe("over");
  });

  it("kalan bütçe eğrisi dönemden önceki harcamayı da sayar", () => {
    // 10 gün bütçe, toplam 7 gün; son üç haftada 1, 2, 1 gün → dönem başında 7 gün kalmıştı.
    expect(burnDown(10 * DAY, 7 * DAY, [DAY, 2 * DAY, DAY]).map((s) => s / DAY)).toEqual([7, 6, 4, 3]);
    expect(burnDown(DAY, 2 * DAY, [])).toEqual([-DAY]);
  });

  it("bitiş tahmini süren haftayı saymaz", () => {
    expect(weeksLeft(4 * DAY, [DAY, 3 * DAY, 9 * DAY])).toBe(2);
    expect(weeksLeft(4 * DAY, [0, 0, 5 * DAY])).toBeNull();
    expect(weeksLeft(0, [DAY, DAY])).toBeNull();
  });
});

describe("arşiv", () => {
  it("seçicilere yalnızca etkin projeler gelir", () => {
    const tags: Tag[] = [
      { id: "a", kind: "project", name: "A", color: 1 },
      { id: "b", kind: "project", name: "B", color: 2, archived: true },
      { id: "c", kind: "category", name: "C", color: 3 },
    ];
    expect(activeProjects(tags).map((t) => t.id)).toEqual(["a"]);
    expect(archivedProjects(tags).map((t) => t.id)).toEqual(["b"]);
  });
});
