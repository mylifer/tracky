import { describe, expect, it } from "vitest";
import type { WorkBlock } from "../api";
import { draftGroups } from "./draft";

const DAY = "2026-11-03T";
const at = (hm: string) => new Date(`${DAY}${hm}:00`).toISOString();
const minutes = (a: string, b: string) => (Date.parse(at(b)) - Date.parse(at(a))) / 60_000;

function blk(from: string, to: string, projectId: string | null, extra: Partial<WorkBlock> = {}): WorkBlock {
  return {
    start: at(from),
    end: at(to),
    activeSeconds: minutes(from, to) * 60,
    categoryId: null,
    projectId,
    switches: 0,
    topApps: [{ appId: "app", appName: "App", seconds: minutes(from, to) * 60 }],
    ...extra,
  };
}

const range = (g: { start: string; end: string }) =>
  `${new Date(g.start).toTimeString().slice(0, 5)}–${new Date(g.end).toTimeString().slice(0, 5)}`;

describe("draftGroups", () => {
  it("eşik 0 ise her blok ayrı kalır", () => {
    const blocks = [blk("09:00", "09:10", "a"), blk("09:12", "09:30", "a")];
    expect(draftGroups(blocks, 0)).toHaveLength(2);
  });

  it("aynı projenin kısa molayla bölünen blokları tek taslak olur", () => {
    const blocks = [blk("04:22", "05:52", "yayin"), blk("06:03", "08:40", "yayin")];
    expect(draftGroups(blocks, 5)).toHaveLength(2);
    const [g] = draftGroups(blocks, 15);
    expect(range(g)).toBe("04:22–08:40");
    expect(g.projectId).toBe("yayin");
    expect(g.blocks).toHaveLength(2);
  });

  it("kısa araya giren başka proje taslağa katılır, araya giren olarak işaretlenir", () => {
    const blocks = [blk("09:00", "09:54", "togg"), blk("09:56", "10:04", "kum"), blk("10:06", "11:00", "togg")];
    const groups = draftGroups(blocks, 15);
    expect(groups).toHaveLength(1);
    expect(groups[0].projectId).toBe("togg");
    expect(groups[0].others.map((b) => b.projectId)).toEqual(["kum"]);
  });

  it("eşikten uzun başka proje ayrı taslak kalır", () => {
    const blocks = [blk("09:00", "10:13", "togg"), blk("10:13", "11:02", "kum"), blk("11:02", "12:09", "togg")];
    expect(draftGroups(blocks, 15).map((g) => g.projectId)).toEqual(["togg", "kum", "togg"]);
  });

  it("projesiz bloklar yanındaki projeli taslağa katılır", () => {
    const blocks = [blk("09:00", "09:40", "togg"), blk("09:42", "10:00", null), blk("10:03", "10:30", "togg")];
    const [g, ...rest] = draftGroups(blocks, 15);
    expect(rest).toHaveLength(0);
    expect(g.projectId).toBe("togg");
  });

  it("eşikten uzun boşluk taslağı böler", () => {
    const blocks = [blk("11:00", "12:09", "togg"), blk("14:23", "14:40", "togg")];
    expect(draftGroups(blocks, 30)).toHaveLength(2);
  });

  it("kısa blok aynı projeli komşuyu tercih eder", () => {
    const blocks = [blk("08:00", "09:00", "a"), blk("09:02", "09:06", "b"), blk("09:08", "10:30", "c")];
    const groups = draftGroups(blocks, 5);
    // 4 dakikalık b, iki komşu da başka projede: uzun olana (c) katılır.
    expect(groups.map((g) => g.projectId)).toEqual(["a", "c"]);
    const same = draftGroups([blk("08:00", "09:00", "a"), blk("09:02", "09:06", "c"), blk("09:08", "10:30", "c")], 5);
    expect(same.map((g) => g.projectId)).toEqual(["a", "c"]);
    expect(same[1].blocks).toHaveLength(2);
  });

  it("gönderilmemiş blokları sayar", () => {
    const blocks = [blk("09:00", "09:30", "a", { sent: true }), blk("09:32", "10:00", "a", { sent: false })];
    const [g] = draftGroups(blocks, 15);
    expect(g.unsent).toBe(1);
    expect(g.sent).toBe(false);
  });

  it("sıralı olmayan girdide de aynı sonucu verir", () => {
    const blocks = [blk("10:06", "11:00", "togg"), blk("09:00", "09:54", "togg"), blk("09:56", "10:04", null)];
    const [g] = draftGroups(blocks, 15);
    expect(range(g)).toBe("09:00–11:00");
    expect(g.blocks.map((b) => b.start)).toEqual([at("09:00"), at("09:56"), at("10:06")]);
  });
});
