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
});
