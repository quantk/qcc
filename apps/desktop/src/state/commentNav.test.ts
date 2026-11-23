import { describe, expect, it } from "vitest";

import type { LineComment } from "../api/protocol.gen";
import { commentStops, stepStop, stopIndex } from "./commentNav";
import { placeComments } from "./comments";
import { parseDiff } from "./diff";

const DIFF = `diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -10,4 +10,4 @@
 fn main() {
-    old();
+    new();
     done();
 }
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1,2 +1,2 @@
-one
+two
 three
`;

function comment(id: string, path: string, line: number, fields: Partial<LineComment> = {}): LineComment {
  return { id, thread_id: "t", path, line, side: "new", text: "x", created_at: 0, ...fields };
}

const files = parseDiff(DIFF);
const ids = (stops: { comment: LineComment }[]) => stops.map((stop) => stop.comment.id);

describe("commentStops", () => {
  it("orders the open comments by file, then down the file, leaving out resolved and unchanged files", () => {
    const comments = [
      comment("b2", "b.rs", 2, { quote: "three" }),
      comment("a13", "a.rs", 13, { quote: "}" }),
      comment("gone", "c.rs", 1),
      comment("done", "a.rs", 10, { resolved_at: 5 }),
      comment("a11", "a.rs", 11, { quote: "    new();" }),
      comment("b1", "b.rs", 1, { quote: "two" }),
    ];
    const stops = commentStops(["a.rs", "b.rs"], comments, placeComments(files, comments));
    expect(ids(stops)).toEqual(["a11", "a13", "b1", "b2"]);
    expect(stops[0]!.where).toEqual({ kind: "line", index: 3 });
  });

  it("follows the order of the changed files, not of the comments' paths", () => {
    const comments = [comment("a", "a.rs", 10, { quote: "fn main() {" }), comment("b", "b.rs", 1, { quote: "two" })];
    expect(ids(commentStops(["b.rs", "a.rs"], comments, placeComments(files, comments)))).toEqual(["b", "a"]);
  });

  it("puts comments adrift above the file's lines, and orders an unread file by line number", () => {
    const comments = [
      comment("line", "a.rs", 10, { quote: "fn main() {" }),
      comment("adrift", "a.rs", 99, { quote: "nowhere" }),
      comment("late", "z.rs", 40),
      comment("early", "z.rs", 3),
    ];
    const placement = placeComments(files, comments, new Set(["a.rs", "b.rs", "z.rs"]));
    const stops = commentStops(["a.rs", "b.rs", "z.rs"], comments, placement);
    expect(ids(stops)).toEqual(["adrift", "line", "early", "late"]);
    expect(stops.map((stop) => stop.where.kind)).toEqual(["head", "line", "unread", "unread"]);
  });
});

describe("stepStop", () => {
  const comments = [
    comment("a", "a.rs", 10, { quote: "fn main() {" }),
    comment("b", "a.rs", 13, { quote: "}" }),
    comment("c", "b.rs", 2, { quote: "three" }),
  ];
  const stops = commentStops(["a.rs", "b.rs"], comments, placeComments(files, comments));
  const place = (id: string) => {
    const stop = stops[stopIndex(stops, id)]!;
    return { id, file: stop.file, at: stop.at };
  };

  it("starts at the first going down and at the last going up", () => {
    expect(stepStop(stops, null, 1)?.comment.id).toBe("a");
    expect(stepStop(stops, null, -1)?.comment.id).toBe("c");
  });

  it("steps to the neighbours and goes round at the ends", () => {
    expect(stepStop(stops, place("a"), 1)?.comment.id).toBe("b");
    expect(stepStop(stops, place("b"), -1)?.comment.id).toBe("a");
    expect(stepStop(stops, place("c"), 1)?.comment.id).toBe("a");
    expect(stepStop(stops, place("a"), -1)?.comment.id).toBe("c");
  });

  it("steps from where a comment that is gone stood", () => {
    const gone = { ...place("b"), id: "resolved" };
    expect(stepStop(stops, gone, 1)?.comment.id).toBe("c");
    expect(stepStop(stops, gone, -1)?.comment.id).toBe("a");
    const pastTheEnd = { id: "x", file: 5, at: 0 };
    expect(stepStop(stops, pastTheEnd, 1)?.comment.id).toBe("a");
  });

  it("has nowhere to go without comments", () => {
    expect(stepStop([], null, 1)).toBeNull();
    expect(stopIndex(stops, null)).toBe(-1);
  });
});
