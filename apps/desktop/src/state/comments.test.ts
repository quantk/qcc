import { describe, expect, it } from "vitest";

import type { LineComment } from "../api/protocol.gen";
import { parseDiff } from "./diff";
import { lineAnchor, placeComments } from "./comments";

const DIFF = `diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,4 +10,4 @@
 fn main() {
-    old();
+    new();
     done();
 }
`;

function comment(id: string, fields: Partial<LineComment>): LineComment {
  return { id, thread_id: "t", path: "src/a.rs", line: 1, side: "new", text: "x", created_at: 0, ...fields };
}

describe("lineAnchor", () => {
  it("counts deleted lines on the old side and the rest on the new", () => {
    const [file] = parseDiff(DIFF);
    expect(file!.lines.map(lineAnchor)).toEqual([
      null,
      { side: "new", line: 10 },
      { side: "old", line: 11 },
      { side: "new", line: 11 },
      { side: "new", line: 12 },
      { side: "new", line: 13 },
    ]);
  });
});

describe("placeComments", () => {
  const files = parseDiff(DIFF);

  it("puts a comment under the line it was left on", () => {
    const placed = placeComments(files, [
      comment("a", { line: 11, quote: "    new();" }),
      comment("b", { line: 11, side: "old", quote: "    old();" }),
      comment("c", { line: 12 }),
    ]);
    const lines = placed.byFile.get("src/a.rs")!;
    expect([...lines.entries()].map(([index, list]) => [index, list.map((c) => c.id)])).toEqual([
      [3, ["a"]],
      [2, ["b"]],
      [4, ["c"]],
    ]);
  });

  it("follows the line's text when the code moved", () => {
    // Left on line 5, which has since moved down to 12.
    const placed = placeComments(files, [comment("a", { line: 5, quote: "    done();" })]);
    expect(placed.byFile.get("src/a.rs")!.get(4)!.map((c) => c.id)).toEqual(["a"]);
  });

  it("keeps apart comments whose line or file is gone", () => {
    const placed = placeComments(files, [
      comment("a", { line: 11, quote: "    gone();" }),
      comment("b", { path: "src/other.rs" }),
    ]);
    expect(placed.byFile.size).toBe(0);
    expect(placed.adrift.get("src/a.rs")!.map((c) => c.id)).toEqual(["a"]);
    expect(placed.elsewhere.map((c) => c.id)).toEqual(["b"]);
  });

  it("waits for the diff of a changed file that is not read yet", () => {
    const placed = placeComments([], [comment("a", { line: 11 }), comment("b", { path: "gone.rs" })], new Set(["src/a.rs"]));
    expect(placed.elsewhere.map((c) => c.id)).toEqual(["b"]);
    expect(placed.byFile.size + placed.adrift.size).toBe(0);
  });
});

describe("placeComments in a file shown whole", () => {
  // The same change as `DIFF`, with the whole file around it.
  const lines = Array.from({ length: 20 }, (_, index) => `line ${index + 1}`);
  const before = [...lines.slice(0, 9), "fn main() {", "    old();", "    done();", "}", ...lines.slice(13)];
  const after = [...lines.slice(0, 9), "fn main() {", "    new();", "    done();", "}", ...lines.slice(13)];
  const whole =
    "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,20 +1,20 @@\n" +
    before.slice(0, 10).map((text) => ` ${text}\n`).join("") +
    "-    old();\n+    new();\n" +
    after.slice(11).map((text) => ` ${text}\n`).join("");

  it("keeps each comment on its line in both views", () => {
    const comments = [
      comment("del", { side: "old", line: 11, quote: "    old();" }),
      comment("add", { line: 11, quote: "    new();" }),
      comment("ctx", { line: 12, quote: "    done();" }),
      comment("far", { line: 3, quote: "line 3" }),
    ];
    const text = (placement: ReturnType<typeof placeComments>, file: ReturnType<typeof parseDiff>[number]) =>
      Object.fromEntries(
        [...(placement.byFile.get("src/a.rs") ?? [])].flatMap(([index, list]) =>
          list.map((c) => [c.id, `${lineAnchor(file.lines[index]!)!.side}:${file.lines[index]!.text}`]),
        ),
      );
    const [hunks] = parseDiff(DIFF);
    const [full] = parseDiff(whole);
    const inHunks = placeComments([hunks!], comments);
    const inFull = placeComments([full!], comments);
    const expected = { del: "old:    old();", add: "new:    new();", ctx: "new:    done();" };
    expect(text(inHunks, hunks!)).toEqual(expected);
    expect(text(inFull, full!)).toEqual({ ...expected, far: "new:line 3" });
    // A line only the whole file shows waits in the file's head for it.
    expect(inHunks.adrift.get("src/a.rs")?.map((c) => c.id)).toEqual(["far"]);
    expect(inFull.adrift.size).toBe(0);
  });
});
