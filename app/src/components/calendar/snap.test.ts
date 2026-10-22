import { describe, expect, it } from "vitest";
import type { Meeting, WindowSpan } from "../../api";
import { snapEdge, snapReach, snapTargets, type SnapTarget } from "./snap";

const MIN = 60_000;
const t = (hm: string) => Date.parse(`2026-10-07T${hm}:00+03:00`);
const iso = (ms: number) => new Date(ms).toISOString();
const day = [t("00:00"), t("00:00") + 24 * 60 * MIN] as const;

const meeting = (from: string, to: string, subject = "Sprint"): Meeting => ({
  uid: subject,
  start: iso(t(from)),
  end: iso(t(to)),
  subject,
  location: "",
  online: true,
  agenda: "",
});

const win = (from: number, to: number, appId = "code", title = "a.ts"): WindowSpan => ({
  start: iso(from),
  end: iso(to),
  appId,
  appName: appId === "code" ? "Visual Studio Code" : appId,
  title,
  categoryId: null,
  projectId: null,
  domain: null,
});

describe("snapTargets", () => {
  it("toplantının başı ve sonu hedeftir", () => {
    const got = snapTargets([meeting("10:00", "10:30")], [], ...day);
    expect(got.map((s) => [s.at, s.kind, s.edge])).toEqual([
      [t("10:00"), "meeting", "start"],
      [t("10:30"), "meeting", "end"],
    ]);
  });

  it("büyük uygulama kümesinin sınırları dakikaya dışa doğru yuvarlanır; kısa kullanım hedef değildir", () => {
    const got = snapTargets(
      [],
      [
        win(t("09:00") + 20_000, t("09:40") + 10_000),
        // Kısa Slack bakışı kümeyi bölmez, kendisi de hedef olmaz.
        win(t("09:40") + 10_000, t("09:42"), "slack"),
        win(t("09:42"), t("10:05") + 1000, "code", "b.ts"),
      ],
      ...day,
    );
    expect(got.map((s) => [s.at, s.edge, s.name])).toEqual([
      [t("09:00"), "start", "Visual Studio Code"],
      [t("10:06"), "end", "Visual Studio Code"],
    ]);
  });

  it("uzun aradan sonra dönülen uygulama yeni kümedir; süresi kısaysa hedef olmaz", () => {
    const got = snapTargets([], [win(t("09:00"), t("09:30")), win(t("10:00"), t("10:10"))], ...day);
    expect(got.map((s) => s.at)).toEqual([t("09:00"), t("09:30")]);
  });

  it("aralıklarla toplamı yeten küme büyüktür; iki bilgisayarda üst üste süre iki kez sayılmaz", () => {
    const spread = [0, 1, 2, 3].map((k) => win(t("09:00") + k * 7 * MIN, t("09:00") + k * 7 * MIN + 4 * MIN));
    expect(snapTargets([], spread, ...day).map((s) => s.at)).toEqual([t("09:00"), t("09:25")]);
    const twice = [win(t("09:00"), t("09:10")), win(t("09:00"), t("09:10"))];
    expect(snapTargets([], twice, ...day)).toEqual([]);
  });

  it("aynı dakikadaki hedeflerden toplantınınki kalır; günün dışındakiler atılır", () => {
    const got = snapTargets(
      [meeting("10:00", "11:00"), { ...meeting("23:30", "23:30"), end: iso(day[1] + 30 * MIN) }],
      [win(t("09:00"), t("10:00"))],
      ...day,
    );
    expect(got.map((s) => [s.at, s.kind])).toEqual([
      [t("09:00"), "app"],
      [t("10:00"), "meeting"],
      [t("11:00"), "meeting"],
      [t("23:30"), "meeting"],
    ]);
  });
});

describe("snapEdge", () => {
  const targets: SnapTarget[] = [
    { at: t("10:00"), kind: "meeting", name: "Sprint", edge: "start" },
    { at: t("10:30"), kind: "app", name: "Code", edge: "end" },
  ];
  const pull = 8 * MIN;
  const release = 14 * MIN;

  it("çekim içindeki en yakın hedefe yapışır, uzaktakine yapışmaz", () => {
    expect(snapEdge(t("10:06"), targets, pull, release, null)?.at).toBe(t("10:00"));
    expect(snapEdge(t("10:24"), targets, pull, release, null)?.at).toBe(t("10:30"));
    expect(snapEdge(t("10:15"), targets, pull, release, null)).toBeNull();
  });

  it("yapışan kenar çekimin ötesinde de kalır, ancak bırakma mesafesini aşınca kurtulur", () => {
    const held = targets[0];
    expect(snapEdge(t("10:12"), targets, pull, release, held)).toBe(held);
    expect(snapEdge(t("10:15"), targets, pull, release, held)).toBeNull();
    // Kurtulduktan sonra öteki hedefin çekimine girer.
    expect(snapEdge(t("10:23"), targets, pull, release, null)?.at).toBe(t("10:30"));
  });

  it("geçersiz hedef atlanır (blok ters dönmesin, bitiş şimdiyi geçmesin)", () => {
    expect(snapEdge(t("10:02"), targets, pull, release, null, (at) => at > t("10:00"))).toBeNull();
    expect(snapEdge(t("10:02"), targets, pull, release, targets[0], (at) => at > t("10:00"))).toBeNull();
  });
});

describe("snapReach", () => {
  it("piksel mesafesi yakınlaştırmayla daralır, uzaklaştırınca üst sınırda kalır", () => {
    expect(snapReach(80).pull).toBe(7.5 * MIN);
    expect(snapReach(320).pull).toBeLessThan(2 * MIN);
    expect(snapReach(10).pull).toBe(15 * MIN);
    expect(snapReach(80).release).toBeGreaterThan(snapReach(80).pull);
  });
});
