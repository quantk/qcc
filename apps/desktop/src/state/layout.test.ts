import { describe, expect, it } from "vitest";

import { SIDEBAR, clampWidth, draggedWidth } from "./layout";

describe("side widths", () => {
  it("stay within bounds", () => {
    expect(clampWidth(100, SIDEBAR)).toBe(SIDEBAR.min);
    expect(clampWidth(5000, SIDEBAR)).toBe(SIDEBAR.max);
    expect(clampWidth(300.6, SIDEBAR)).toBe(301);
  });

  it("fall back to the initial width for anything that is not a number", () => {
    expect(clampWidth(undefined, SIDEBAR)).toBe(SIDEBAR.initial);
    expect(clampWidth("300", SIDEBAR)).toBe(SIDEBAR.initial);
    expect(clampWidth(Number.NaN, SIDEBAR)).toBe(SIDEBAR.initial);
  });

  it("follow the pointer from the edge being dragged", () => {
    expect(draggedWidth(280, 280, 330, "right", SIDEBAR)).toBe(330);
    expect(draggedWidth(300, 900, 850, "left", SIDEBAR)).toBe(350);
    expect(draggedWidth(300, 900, 0, "left", SIDEBAR)).toBe(SIDEBAR.max);
  });
});
