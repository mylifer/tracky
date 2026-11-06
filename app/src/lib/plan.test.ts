import { describe, expect, it } from "vitest";
import type { WindowSpan } from "../api";
import { NO_PROJECT } from "../api";
import { buildModel, guess, relabel } from "./learn";
import {
  confirm,
  draftSlots,
  draw,
  meetingSlots,
  mergeWith,
  normalize,
  PLAN_DEFAULTS,
  remove,
  resize,
  type Slot,
  split,
  update,
  visibleSuggestions,
  withAssigned,
} from "./plan";

const DAY = Date.parse("2026-11-03T00:00:00");
const at = (hm: string) => {
  const [h, m] = hm.split(":").map(Number);
  return DAY + (h * 60 + m) * 60_000;
};
const hm = (t: number) => new Date(t).toTimeString().slice(0, 5);
const NOW = at("14:40");

function win(from: string, to: string, projectId: string | null, appName = "App"): WindowSpan {
  return {
    start: new Date(at(from)).toISOString(),
    end: new Date(at(to)).toISOString(),
    appId: appName,
    appName,
    title: appName,
    categoryId: null,
    projectId,
    domain: null,
  };
}

const show = (list: Slot[]) => list.map((b) => `${hm(b.start)}–${hm(b.end)} ${b.projectId ?? "-"}`);

// Ekran görüntüsündeki günün kabası: gece oyun, sabah yayın (arada 11 dk kalkış), SD App,
// arada 44 dk Kum, öğle arası, öğleden sonra Kum.
const DAY_WINDOWS = [
  win("02:28", "02:44", null, "Discord"),
  win("02:50", "03:17", "oyun"),
  win("03:17", "03:23", "oyun"),
  win("03:23", "03:27", null, "Önizleme"),
  win("03:30", "04:01", "oyun"),
  win("04:01", "04:12", null, "Claude"),
  win("04:12", "04:16", "kum"),
  win("04:16", "04:21", "togg"),
  win("04:22", "05:52", "yayin"),
  win("06:03", "06:50", "yayin"),
  win("06:50", "06:57", null, "Claude"),
  win("06:57", "07:01", "togg"),
  win("07:01", "08:40", "yayin"),
  win("09:00", "09:54", "togg"),
  win("10:00", "10:04", null, "WhatsApp"),
  win("10:13", "10:36", "kum"),
  win("10:36", "10:44", "togg"),
  win("10:44", "10:57", "kum"),
  win("10:57", "11:06", null, "Claude"),
  win("11:06", "12:09", "togg"),
  win("14:23", "14:40", "kum"),
];

describe("taslak", () => {
  it("günü birkaç büyük bloğa indirir; kısa kalkış ve araya girmeler blok açmaz", () => {
    const drafts = draftSlots(DAY_WINDOWS, DAY, DAY + 86_400_000, NOW, []);
    expect(show(drafts)).toEqual([
      "02:30–04:15 oyun",
      "04:15–08:40 yayin",
      "09:00–10:15 togg",
      "10:15–11:00 kum",
      "11:00–12:10 togg",
      "14:20–14:40 kum",
    ]);
    expect(drafts.every((b) => b.draft)).toBe(true);
  });

  it("onaylı blokların aralığına girmez", () => {
    const confirmed: Slot[] = [{ id: "a", start: at("09:00"), end: at("12:10"), projectId: "togg", note: "" }];
    const drafts = draftSlots(DAY_WINDOWS, DAY, DAY + 86_400_000, NOW, confirmed);
    expect(drafts.some((b) => b.end > at("09:00") && b.start < at("12:10"))).toBe(false);
  });

  it("atanmamış süre yandaki projeli bloğu uzatmaz", () => {
    // 09:00–09:30 Trumore, ardından 45 dk atanmamış tarayıcı, sonra yine Trumore.
    const w = [win("09:00", "09:30", "tru"), win("09:30", "10:15", null, "Safari"), win("10:15", "11:00", "tru")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual([
      "09:00–09:30 tru",
      "09:30–10:15 -",
      "10:15–11:00 tru",
    ]);
  });

  it("şu an süren atanmamış iş projeli bloğa katılmaz (blok dakika dakika büyümez)", () => {
    // 14:00–14:20 Trumore, 14:20'den beri (şimdi 14:31) atanmamış.
    const w = [win("13:00", "14:20", "tru"), win("14:20", "14:31", null, "Safari")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, at("14:31"), []))).toEqual(["13:00–14:15 tru", "14:15–14:30 -"]);
  });

  it("bir dilimlik atanmamış araya girme önceki blokta kalır", () => {
    const w = [win("09:00", "10:00", "tru"), win("10:00", "10:12", null, "WhatsApp"), win("10:15", "11:00", "kum")];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual(["09:00–10:15 tru", "10:15–11:00 kum"]);
  });

  it("aynı projede iş değişince ayrı blok açar", () => {
    // Bütün sabah Trumore: önce Figma'da Onboarding UI/UX, sonra Pitch deck.
    const w = [
      { ...win("09:00", "10:30", "tru", "Figma"), title: "Trumore Onboarding UI/UX" },
      { ...win("10:30", "11:30", "tru", "Figma"), title: "Trumore Pitch Deck" },
    ];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual(["09:00–10:30 tru", "10:30–11:30 tru"]);
  });

  it("sekmeden sekmeye geçilen karışık süre ve kısa iş değişikliği bloğu parçalamaz", () => {
    // 10 dakikada bir başlık değişiyor (baskın başlık yok) ve araya 15 dk başka iş giriyor.
    const titles = ["A", "B", "C", "D", "E", "F"];
    const chatter = titles.map((t, k) => ({
      ...win(`09:${String(k * 10).padStart(2, "0")}`, `09:${String(k * 10 + 10).padStart(2, "0")}`, "tru"),
      title: t,
    }));
    const w = [
      { ...win("08:00", "09:00", "tru"), title: "Onboarding" },
      ...chatter.slice(0, 5),
      { ...win("09:50", "10:00", "tru"), title: "Onboarding" },
      { ...win("10:00", "10:15", "tru"), title: "Kısa iş" },
      { ...win("10:15", "11:00", "tru"), title: "Onboarding" },
    ];
    expect(show(draftSlots(w, DAY, DAY + 86_400_000, NOW, []))).toEqual(["08:00–11:00 tru"]);
  });

  it("şimdiden sonrasını çizmez", () => {
    const drafts = draftSlots([win("14:00", "15:00", "togg")], DAY, DAY + 86_400_000, at("14:30"), []);
    expect(show(drafts)).toEqual(["14:00–14:30 togg"]);
  });
});

const b = (id: string, from: string, to: string, projectId: string | null = "p"): Slot => ({
  id,
  start: at(from),
  end: at(to),
  projectId,
  note: "",
});

// Her test docs/gun-plani.md'deki bir kurala karşılık gelir.
describe("kurallar", () => {
  const meeting = (from: string, to: string, extra: Partial<{ projectId: string | null; ignored: boolean }> = {}) => ({
    uid: `m${from}`,
    start: new Date(at(from)).toISOString(),
    end: new Date(at(to)).toISOString(),
    subject: "Haftalık toplantı",
    ...extra,
  });

  it("1, V7: takvim etkinliği tam saatinde, toplantının projesiyle kesik öneri olur", () => {
    const out = meetingSlots([meeting("10:00", "10:45", { projectId: "togg" })], DAY, DAY + 86_400_000, [], new Set());
    expect(out.map((x) => `${hm(x.start)}–${hm(x.end)} ${x.projectId} ${x.note} ${x.draft}`)).toEqual([
      "10:00–10:45 togg Haftalık toplantı true",
    ]);
    // Onaylanınca öneri olmaktan çıkar.
    expect(confirm(out[0])).toMatchObject({ draft: undefined, meeting: undefined, projectId: "togg" });
    expect(confirm(out[0]).id).not.toBe(out[0].id);
  });

  it("1, V3: yok sayılan, silinen ya da onaylı blokla çakışan etkinlik öneri olmaz", () => {
    const ms = [meeting("09:00", "09:30", { ignored: true }), meeting("10:00", "10:30"), meeting("11:00", "11:30")];
    const dismissed = new Set([`${ms[1].uid}@${ms[1].start}`]);
    const confirmed = [b("x", "11:15", "12:00")];
    expect(meetingSlots(ms, DAY, DAY + 86_400_000, confirmed, dismissed)).toEqual([]);
  });

  it("2: etkinlik saatinde bilgisayar işi ayrı öneri açmaz", () => {
    const ms = meetingSlots([meeting("10:00", "11:00")], DAY, DAY + 86_400_000, [], new Set());
    const drafts = draftSlots([win("09:00", "12:00", "kum")], DAY, DAY + 86_400_000, NOW, ms);
    expect(drafts.some((d) => d.start < at("11:00") && at("10:00") < d.end)).toBe(false);
    // Saklanmış öneriler de etkinliğin saatinde görünmez.
    const stored = [{ ...b("d", "09:00", "12:00", "kum"), draft: true }];
    expect(show(visibleSuggestions(stored, [], ms))).toEqual(["09:00–10:00 kum", "11:00–12:00 kum"]);
  });

  it("3a: öneri modeli geçmişte onaylanan bloklardan proje ve ad öğrenir", () => {
    const m = buildModel([
      { appName: "Figma", title: "Onboarding – Figma", projectId: "trumore", note: "Onboarding UI/UX", seconds: 3600 },
      { appName: "Code", title: "plan.ts — kum", projectId: "kum", note: "Gün planı", seconds: 5400 },
      { appName: "Code", title: "api.rs — kum", projectId: "kum", note: "Gün planı", seconds: 1800 },
    ]);
    expect(guess(m, "Figma", "Onboarding – Figma")).toEqual({ projectId: "trumore", note: "Onboarding UI/UX" });
    // Görülmemiş başlık: kelimelerden ("kum").
    expect(guess(m, "Code", "lib.rs — kum")).toEqual({ projectId: "kum", note: "Gün planı" });
    // Hiç görülmemiş uygulama ve kelimeler: tahmin yok.
    expect(guess(m, "Safari", "Hava durumu")).toBeNull();
    const w = { ...win("09:00", "10:00", null, "Figma"), title: "Onboarding – Figma" };
    const out = relabel(m, [w]);
    expect(out.guessed).toBe(1);
    expect(out.windows[0]).toMatchObject({ projectId: "trumore", title: "Onboarding UI/UX" });
    // Tahmin edilen ad önerinin adı olur.
    const drafts = draftSlots(out.windows, DAY, DAY + 86_400_000, NOW, [], { ...PLAN_DEFAULTS, names: out.names });
    expect(drafts.map((d) => `${d.projectId} ${d.note}`)).toEqual(["trumore Onboarding UI/UX"]);
  });

  it("3b: geçmiş yoksa basit öneriler: adsız, projeye göre, uzun boşlukta bölünür", () => {
    const m = buildModel([]);
    const windows = [win("09:00", "10:00", "kum"), win("11:00", "12:00", "kum")];
    const out = relabel(m, windows);
    expect(out.guessed).toBe(0);
    const drafts = draftSlots(out.windows, DAY, DAY + 86_400_000, NOW, [], { ...PLAN_DEFAULTS, names: out.names });
    expect(drafts.map((d) => `${hm(d.start)}–${hm(d.end)} ${d.projectId} "${d.note}"`)).toEqual([
      '09:00–10:00 kum ""',
      '11:00–12:00 kum ""',
    ]);
  });

  it("3c: Pencereler'de projeye atanan aralık o projeyle öneri olur, yakın aynı projeli öneriyle birleşir", () => {
    const end = DAY + 86_400_000;
    // Figma'da 09:18–11:43 Onboarding: Trumore'a atandı.
    const one = withAssigned([], at("09:18"), at("11:43"), "trumore", DAY, end);
    expect(show(one)).toEqual(["09:15–11:45 trumore"]);
    expect(one[0].draft).toBe(true);
    // Başka projeli öneri olan aralığa atama onu ezer; kalan kısmı durur.
    const other = [{ ...b("x", "11:00", "13:00", "kum"), draft: true }];
    expect(show(withAssigned(other, at("09:18"), at("11:43"), "trumore", DAY, end))).toEqual([
      "09:15–11:45 trumore",
      "11:45–13:00 kum",
    ]);
    // Aynı projede peş peşe atanan pencereler tek öneri.
    const two = withAssigned(one, at("11:50"), at("12:30"), "trumore", DAY, end);
    expect(show(two)).toEqual(["09:15–12:30 trumore"]);
    // Projesiz (iş değil) atama aralıktaki öneriyi kaldırır.
    expect(show(withAssigned(two, at("12:00"), at("12:30"), NO_PROJECT, DAY, end))).toEqual(["09:15–12:00 trumore"]);
  });

  it("4: küçültmek ya da silmek boşluk açar; komşu ve öneri oraya gelmez", () => {
    const list = [b("ust", "09:00", "10:00"), b("alt", "10:00", "11:00")];
    expect(show(resize(list, "alt", "start", at("10:30"), NOW))).toEqual(["09:00–10:00 p", "10:30–11:00 p"]);
    const three = [...list, b("son", "11:00", "12:00", "q")];
    expect(show(resize(three, "alt", "end", at("10:40"), NOW))).toEqual([
      "09:00–10:00 p",
      "10:00–10:40 p",
      "11:00–12:00 q",
    ]);
    expect(show(remove(three, "alt"))).toEqual(["09:00–10:00 p", "11:00–12:00 q"]);
  });

  it("5: büyütülen blok yalnızca kendisi değişir, komşunun kenarında durur, şimdiyi geçmez", () => {
    const list = [b("a", "09:00", "10:00"), b("b", "10:00", "11:00", "q")];
    expect(show(resize(list, "a", "end", at("10:20"), NOW))).toEqual(["09:00–10:00 p", "10:00–11:00 q"]);
    const gap = [b("a", "09:00", "10:00"), b("b", "10:30", "11:00", "q")];
    expect(show(resize(gap, "a", "end", at("10:45"), NOW))).toEqual(["09:00–10:30 p", "10:30–11:00 q"]);
    expect(show(resize(gap, "b", "start", at("09:30"), NOW))).toEqual(["09:00–10:00 p", "10:00–11:00 q"]);
    expect(show(resize(gap, "b", "end", at("16:00"), NOW))).toEqual(["09:00–10:00 p", "10:30–14:40 q"]);
  });

  it("6: aynı projede art arda bloklar ayrı kalır; ad ya da proje vermek birleştirmez", () => {
    const list = [b("a", "06:25", "07:30"), b("c", "07:30", "08:50")];
    expect(show(update(list, "c", { note: "API entegrasyonu" }))).toEqual(["06:25–07:30 p", "07:30–08:50 p"]);
    expect(show(update([b("a", "06:25", "07:30"), b("c", "07:30", "08:50", "q")], "c", { projectId: "p" }))).toEqual([
      "06:25–07:30 p",
      "07:30–08:50 p",
    ]);
    expect(show(normalize([b("d", "10:00", "11:00"), b("c", "10:00", "10:00"), b("a", "09:00", "10:00")]))).toEqual([
      "09:00–10:00 p",
      "10:00–11:00 p",
    ]);
  });

  it("7: çizim yalnızca boş alana olur, onaylı bloğun kenarında durur", () => {
    const list = [b("a", "09:00", "10:00"), b("c", "11:00", "12:00", "q")];
    expect(show(draw(list, at("09:30"), at("11:30"), "x"))).toEqual([
      "09:00–10:00 p",
      "10:00–11:00 x",
      "11:00–12:00 q",
    ]);
    // Boş yer yoksa hiçbir şey değişmez.
    expect(draw(list, at("09:10"), at("09:50"), "x")).toBe(list);
    // Komşusuyla aynı proje olsa da ayrı blok.
    expect(show(draw(list, at("10:00"), at("11:00"), "p"))).toEqual([
      "09:00–10:00 p",
      "10:00–11:00 p",
      "11:00–12:00 q",
    ]);
  });

  it("8: bölme iki parça verir; birleştirme yalnızca istenen iki bloğu birleştirir", () => {
    const parts = split([b("a", "09:00", "11:00")], "a", at("10:00"));
    expect(show(parts)).toEqual(["09:00–10:00 p", "10:00–11:00 p"]);
    expect(split([b("a", "09:00", "11:00")], "a", at("09:02"))).toHaveLength(1);
    const merged = mergeWith(
      [b("a", "09:00", "09:30", "x"), b("c", "09:40", "11:00"), b("z", "11:00", "12:00")],
      "a",
      "next",
    );
    expect(show(merged)).toEqual(["09:00–11:00 p", "11:00–12:00 p"]);
  });
});
