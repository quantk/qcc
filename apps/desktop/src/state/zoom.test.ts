import { describe, expect, it } from "vitest";

import { clampZoom, formatZoom, loadZoom, saveZoom, zoomIn, zoomOut } from "./zoom";

describe("zoom", () => {
  it("steps through the levels and stops at the ends", () => {
    expect(zoomIn(1)).toBe(1.1);
    expect(zoomOut(1)).toBe(0.9);
    expect(zoomIn(2)).toBe(2);
    expect(zoomOut(0.5)).toBe(0.5);
  });

  it("moves a scale between steps to the nearest step in that direction", () => {
    expect(zoomIn(1.2)).toBe(1.25);
    expect(zoomOut(1.2)).toBe(1.1);
  });

  it("clamps and formats", () => {
    expect(clampZoom(10)).toBe(2);
    expect(clampZoom(Number.NaN)).toBe(1);
    expect(formatZoom(0.67)).toBe("67%");
  });

  it("survives storage that is missing, broken or holds garbage", () => {
    const values = new Map<string, string>();
    const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => void values.set(key, value) };
    expect(loadZoom(storage)).toBe(1);
    saveZoom(storage, 1.25);
    expect(loadZoom(storage)).toBe(1.25);
    values.set("qcc.zoom", "lots");
    expect(loadZoom(storage)).toBe(1);
    expect(loadZoom(undefined)).toBe(1);
    const throwing = {
      getItem: () => {
        throw new Error("denied");
      },
    };
    expect(loadZoom(throwing)).toBe(1);
  });
});
