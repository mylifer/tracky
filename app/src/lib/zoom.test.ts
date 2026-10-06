import { describe, expect, it } from "vitest";
import { CAL_ZOOM_MIN, clampZoom, stepZoom, ZOOM_MAX, ZOOM_MIN } from "./zoom";

describe("yakınlaştırma adımları", () => {
  it("bir sonraki ve önceki adıma gider", () => {
    expect(stepZoom(1, 1)).toBe(1.5);
    expect(stepZoom(1.5, -1)).toBe(1);
    // Hareketle ara bir değerdeyken en yakın adıma.
    expect(stepZoom(2.4, 1)).toBe(3);
    expect(stepZoom(2.4, -1)).toBe(2);
  });

  it("sınırları aşmaz", () => {
    expect(stepZoom(ZOOM_MAX, 1)).toBe(ZOOM_MAX);
    expect(stepZoom(ZOOM_MIN, -1)).toBe(ZOOM_MIN);
    expect(clampZoom(0.2)).toBe(ZOOM_MIN);
    expect(clampZoom(40)).toBe(ZOOM_MAX);
  });

  it("takvim %100'ün altına uzaklaşır", () => {
    expect(stepZoom(1, -1, CAL_ZOOM_MIN)).toBe(0.75);
    expect(stepZoom(0.6, -1, CAL_ZOOM_MIN)).toBe(0.5);
    expect(stepZoom(0.6, 1, CAL_ZOOM_MIN)).toBe(0.75);
    expect(stepZoom(CAL_ZOOM_MIN, -1, CAL_ZOOM_MIN)).toBe(CAL_ZOOM_MIN);
    expect(clampZoom(0.1, CAL_ZOOM_MIN)).toBe(CAL_ZOOM_MIN);
    expect(clampZoom(0.4, CAL_ZOOM_MIN)).toBe(0.4);
  });
});
