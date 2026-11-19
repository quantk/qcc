import { describe, expect, it } from "vitest";

import type { Revision } from "../api/protocol.gen";
import { ALL, scopeHolds, scopeLabel, scopeOfSpan, spanBetween, spanOfScope } from "./changeScope";

// Newest first, as the environment lists them: rows 1, 2 and 3 of the picker.
const revisions: Revision[] = [
  { id: "ccc3333", title: "Address review" },
  { id: "bbb2222", title: "Add tests" },
  { id: "aaa1111", title: "Add the parser" },
];

describe("change scopes", () => {
  it("turns a run of rows into a scope and back", () => {
    const cases = [
      [{ newest: 0, oldest: 0 }, { kind: "unsaved" }],
      [{ newest: 0, oldest: 3 }, ALL],
      [{ newest: 1, oldest: 1 }, { kind: "revisions", from: "bbb2222", to: "ccc3333" }],
      [{ newest: 3, oldest: 3 }, { kind: "revisions", to: "aaa1111" }],
      [{ newest: 1, oldest: 2 }, { kind: "revisions", from: "aaa1111", to: "ccc3333" }],
      [{ newest: 0, oldest: 1 }, { kind: "revisions", from: "bbb2222" }],
    ] as const;
    for (const [span, scope] of cases) {
      expect(scopeOfSpan(span, revisions)).toEqual(scope);
      expect(spanOfScope(scope, revisions)).toEqual(span);
    }
  });

  it("orders a shift-click either way", () => {
    expect(spanBetween(3, 1)).toEqual({ newest: 1, oldest: 3 });
    expect(spanBetween(0, 2)).toEqual({ newest: 0, oldest: 2 });
  });

  it("lets go of commits that are no longer listed", () => {
    expect(scopeHolds({ kind: "revisions", to: "ccc3333", from: "bbb2222" }, revisions)).toBe(true);
    expect(scopeHolds({ kind: "revisions", to: "dead000" }, revisions)).toBe(false);
    expect(scopeHolds({ kind: "revisions", from: "dead000" }, revisions)).toBe(false);
    // The ends the wrong way round.
    expect(scopeHolds({ kind: "revisions", from: "ccc3333", to: "aaa1111" }, revisions)).toBe(false);
    expect(scopeHolds({ kind: "unsaved" }, [])).toBe(true);
  });

  it("names what is shown", () => {
    expect(scopeLabel(ALL, revisions)).toBe("All changes");
    expect(scopeLabel({ kind: "unsaved" }, revisions)).toBe("Uncommitted");
    expect(scopeLabel({ kind: "revisions", from: "bbb2222", to: "ccc3333" }, revisions)).toBe("ccc3333 Address review");
    expect(scopeLabel({ kind: "revisions", from: "aaa1111", to: "ccc3333" }, revisions)).toBe("2 commits");
    expect(scopeLabel({ kind: "revisions", from: "aaa1111" }, revisions)).toBe("2 commits and uncommitted");
  });
});
