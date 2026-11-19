import { describe, expect, it } from "vitest";

import { noteUiChange, uiChangeCount } from "./uiChanges";

describe("plugin view changes", () => {
  it("count per view and per thread's copy", () => {
    let changes = noteUiChange({}, { plugin: "reviewers", view: "reviewers", thread_id: "thr_1" });
    expect(uiChangeCount(changes, "reviewers", "reviewers", "thr_1")).toBe(1);
    expect(uiChangeCount(changes, "reviewers", "reviewers", "thr_2")).toBe(0);
    changes = noteUiChange(changes, { plugin: "reviewers", view: "reviewers" });
    expect(uiChangeCount(changes, "reviewers", "reviewers", "thr_1")).toBe(2);
    expect(uiChangeCount(changes, "reviewers", "reviewers", "thr_2")).toBe(1);
    expect(uiChangeCount(changes, "reviewers", "reviewers")).toBe(1);
    expect(uiChangeCount(changes, "other", "reviewers", "thr_1")).toBe(0);
  });
});
