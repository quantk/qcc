import { describe, expect, it } from "vitest";

import { previewText } from "./preview";

const numbered = (count: number) => Array.from({ length: count }, (_, index) => `line ${index + 1}`).join("\n");
const limits = { lines: 12, chars: 1600 };

describe("previewText", () => {
  it("shows a short output whole", () => {
    expect(previewText("one\ntwo\n", limits)).toEqual({ text: "one\ntwo\n", lines: 2, clipped: false });
  });

  it("shows whole an output only a few lines over the limit", () => {
    expect(previewText(numbered(15), limits)).toEqual({ text: numbered(15), lines: 15, clipped: false });
  });

  it("keeps the first lines of a long output and counts them all", () => {
    expect(previewText(`${numbered(300)}\n`, limits)).toEqual({ text: numbered(12), lines: 300, clipped: true });
  });

  it("cuts a single long line so it does not wrap into a wall", () => {
    const preview = previewText("x".repeat(50_000), limits);
    expect(preview).toEqual({ text: "x".repeat(1600), lines: 1, clipped: true });
  });

  it("stops at the character budget when lines are long", () => {
    const line = "y".repeat(700);
    const preview = previewText(Array(20).fill(line).join("\n"), limits);
    expect(preview.text).toBe([line, line, "y".repeat(1600 - 2 * 701)].join("\n"));
    expect(preview.lines).toBe(20);
  });

  it("is a prefix of the output, so the highlighting carries on when all is shown", () => {
    const text = numbered(400);
    expect(text.startsWith(previewText(text, limits).text)).toBe(true);
  });
});
