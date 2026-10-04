import { describe, expect, it } from "vitest";
import type { Tag } from "../api";
import { projectGroups, readRecent, withRecent } from "./projectOptions";

const tag = (id: string, name: string): Tag => ({ id, kind: "project", name, color: 1 });
const projects = [tag("a", "Portal"), tag("b", "Mobil"), tag("c", "İç araçlar"), tag("d", "Bakım"), tag("e", "Web")];
const many = [...projects, tag("f", "Sunum"), tag("g", "Eğitim")];
const clients = [
  { id: "z", name: "Zeta" },
  { id: "x", name: "Acme" },
];
const projectClients = { a: "z", b: "x", d: "x" };
const ids = (g: { projects: Tag[] }) => g.projects.map((p) => p.id);

describe("proje seçicinin grupları", () => {
  it("müşteri yoksa tek, başlıksız grup", () => {
    expect(projectGroups(projects)).toEqual([{ label: null, projects }]);
  });

  it("müşteriye göre (adına göre sıralı) gruplar, müşterisizler sonda", () => {
    const groups = projectGroups(projects, { clients, projectClients });
    expect(groups.map((g) => g.label)).toEqual(["Acme", "Zeta", "Müşterisiz"]);
    expect(groups.map(ids)).toEqual([["b", "d"], ["a"], ["c", "e"]]);
  });

  it("son kullanılanlar en üstte; liste kısaysa gösterilmez", () => {
    expect(projectGroups(projects, { recent: ["c"] })).toHaveLength(1);
    const groups = projectGroups(many, { recent: ["c", "silinmiş", "a"] });
    expect(groups[0]).toEqual({ label: "Son kullanılanlar", projects: [many[2], many[0]] });
    expect(groups[1].label).toBe("Tüm projeler");
  });

  it("süzme proje ya da müşteri adında arar, seçili proje kalır", () => {
    const groups = projectGroups(many, { clients, projectClients, recent: ["a"], filter: "acme", keep: "e" });
    expect(groups.map(ids)).toEqual([["b", "d"], ["e"]]);
    // Türkçe büyük/küçük harf: "İç" → "iç".
    expect(projectGroups(many, { filter: "iç" }).map(ids)).toEqual([["c"]]);
    expect(projectGroups(many, { filter: "yok" })).toEqual([]);
  });

  it("son kullanılanlar en çok beş, tekrar seçilen başa gelir", () => {
    expect(withRecent(["a", "b", "c", "d", "e"], "f")).toEqual(["f", "a", "b", "c", "d"]);
    expect(withRecent(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
    // Depolama yoksa boş liste (hata fırlatmaz).
    expect(readRecent()).toEqual([]);
  });
});
