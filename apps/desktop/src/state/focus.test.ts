import { describe, expect, it } from "vitest";

import type { DiffFile } from "./diff";
import { focusRow } from "./focus";

const file: DiffFile = {
  path: "a.rs",
  status: "modified",
  binary: false,
  additions: 1,
  deletions: 1,
  lines: [
    { kind: "hunk", text: "@@ -1,2 +1,2 @@" },
    { kind: "context", text: "fn a() {", oldNo: 1, newNo: 1 },
    { kind: "del", text: "old();", oldNo: 2 },
    { kind: "add", text: "new();", newNo: 2 },
  ],
};

describe("a place in the changes", () => {
  const focus = (line: number | null) => ({ threadId: "t", path: "a.rs", line, seq: 1 });
  it("is the row of the line as it is now", () => {
    expect(focusRow(focus(2), file)).toBe("l:a.rs:3");
    expect(focusRow(focus(1), file)).toBe("l:a.rs:1");
  });
  it("is the file's header for the file, or a line the diff does not show", () => {
    expect(focusRow(focus(null), undefined)).toBe("f:a.rs");
    expect(focusRow(focus(40), file)).toBe("f:a.rs");
  });
  it("waits while the file is not read", () => {
    expect(focusRow(focus(2), undefined)).toBeNull();
  });
});
