import { describe, expect, it } from "vitest";

import type { ChangedFile } from "../api/protocol.gen";
import { type ChangesState, EMPTY, reading, withChanges, withDiff } from "./changes";
import { parseDiff } from "./diff";
import { diffExtent, diffRows, firstChange, lineColumns, wantedPaths } from "./diffRows";

function changed(path: string, fields: Partial<ChangedFile> = {}): ChangedFile {
  return { path, kind: "modified", additions: 1, deletions: 1, ...fields };
}

const files = [
  changed("a.ts"),
  changed("big.ts", { additions: 900 }),
  changed("img.png", { additions: null, deletions: null }),
  changed("b.ts"),
];

function readA(): ChangesState {
  const state = withChanges(EMPTY, { files, branch: null, base: null });
  return withDiff(reading(state, ["a.ts"]), ["a.ts"], {
    diff: "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-\tx\n+yy\n",
  });
}

const shape = (rows: ReturnType<typeof diffRows>) =>
  rows.map((row) =>
    row.kind === "line" || row.kind === "below"
      ? `${row.kind}:${row.path}:${row.index}`
      : row.kind === "note"
        ? `note:${row.path}:${row.note}`
        : `${row.kind}:${row.path}`,
  );

describe("diffRows", () => {
  it("lays out read files line by line, and notes for the rest", () => {
    const rows = diffRows({ files, diffs: readA().diffs, opened: new Map() });
    expect(shape(rows)).toEqual([
      "file:a.ts",
      "line:a.ts:0",
      "line:a.ts:1",
      "line:a.ts:2",
      "end:a.ts",
      "file:big.ts",
      "note:big.ts:folded",
      "end:big.ts",
      "file:img.png",
      "note:img.png:binary",
      "end:img.png",
      "file:b.ts",
      "note:b.ts:reading",
      "end:b.ts",
    ]);
  });

  it("follows what the user opened and folded, and what the view adds", () => {
    const rows = diffRows({
      files,
      diffs: readA().diffs,
      opened: new Map([
        ["big.ts", true],
        ["b.ts", false],
      ]),
      heads: new Set(["a.ts"]),
      below: new Map([["a.ts", new Set([1])]]),
    });
    expect(shape(rows).slice(0, 9)).toEqual([
      "file:a.ts",
      "head:a.ts",
      "line:a.ts:0",
      "line:a.ts:1",
      "below:a.ts:1",
      "line:a.ts:2",
      "end:a.ts",
      "file:big.ts",
      "note:big.ts:reading",
    ]);
    expect(shape(rows).slice(-3)).toEqual(["file:b.ts", "note:b.ts:folded", "end:b.ts"]);
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
  });

  it("wants the open files with rows in range, once each and in order", () => {
    const rows = diffRows({ files, diffs: readA().diffs, opened: new Map() });
    expect(wantedPaths(rows, 0, rows.length - 1)).toEqual(["a.ts", "img.png", "b.ts"]);
    // From the middle of a file on: that file too, and not the folded one.
    expect(wantedPaths(rows, 2, 7)).toEqual(["a.ts"]);
  });
});

describe("how wide a diff is", () => {
  it("counts tabs to the next stop", () => {
    expect(lineColumns("\tx")).toBe(5);
    expect(lineColumns("ab\tc", 4)).toBe(5);
    expect(lineColumns("plain")).toBe(5);
  });

  it("measures the open files that are read", () => {
    const state = readA();
    const rows = diffRows({ files, diffs: state.diffs, opened: new Map() });
    const fileOf = (path: string) => {
      const diff = state.diffs.get(path);
      return diff?.state === "read" ? diff.file : undefined;
    };
    expect(diffExtent(rows, fileOf)).toEqual({ columns: 5, digits: 3 });
  });
});

describe("firstChange", () => {
  it("finds the first added or removed line, past the context before it", () => {
    const [whole] = parseDiff("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,4 +1,4 @@\n one\n two\n-three\n+3\n four\n");
    expect(firstChange(whole!)).toBe(3);
    const [same] = parseDiff("diff --git a/a b/a\nold mode 100644\nnew mode 100755\n");
    expect(firstChange(same ?? { path: "a", status: "modified", binary: false, additions: 0, deletions: 0, lines: [] })).toBeNull();
  });
});
