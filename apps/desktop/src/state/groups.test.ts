import { describe, expect, it } from "vitest";

import type { Project, Thread } from "../api/protocol.gen";
import { groupKeys, groupSummary, pruneFolded, summaryText, visibleRows, withFolded } from "./groups";
import { threadTree } from "./selectors";

function thread(id: string, extra: Partial<Thread> = {}): Thread {
  return {
    id,
    provider: "echo",
    cwd: "/work",
    parent: null,
    status: "idle",
    created_at: 1,
    updated_at: 1,
    last_seq: 1,
    ...extra,
  } as Thread;
}

describe("group summary", () => {
  const threads = [
    thread("waiting", { pending_interactions: [{}] } as Partial<Thread>),
    thread("waiting2", { pending_interactions: [{}] } as Partial<Thread>),
    thread("active", { status: "active" }),
    thread("failed", { last_turn_status: "failed" }),
    thread("new", { last_turn_at: 5 }),
    thread("read", { last_turn_at: 5 }),
    thread("archived", { archived: true, status: "failed" }),
  ];

  it("counts what each thread needs, once", () => {
    expect(groupSummary(threads, { read: 5 })).toEqual({ threads: 7, waiting: 2, active: 1, failed: 1, unread: 1 });
  });

  it("says it in words, leaving out what is not there", () => {
    expect(summaryText(groupSummary(threads, { read: 5 }))).toBe(
      "7 threads, 2 waiting for your answer, 1 failed, 1 working, 1 with a new reply",
    );
    expect(summaryText(groupSummary([thread("a")], {}))).toBe("1 thread");
  });
});

describe("folded groups", () => {
  const rows = threadTree([thread("a", { created_at: 1 }), thread("b", { created_at: 2 }), thread("a1", { parent: "a", created_at: 3 })]);

  it("show every row unfolded", () => {
    expect(visibleRows(rows, false, null).map((row) => row.thread.id)).toEqual(["b", "a", "a1"]);
  });

  it("show only the selected thread folded, on its own", () => {
    expect(visibleRows(rows, true, null)).toEqual([]);
    expect(visibleRows(rows, true, "elsewhere")).toEqual([]);
    const row = visibleRows(rows, true, "a1")[0]!;
    expect(row.thread.id).toBe("a1");
    expect(row.depth).toBe(0);
    expect(visibleRows(rows, true, "a")[0]!.children).toBe(0);
  });

  it("fold and unfold several at once", () => {
    const folded = withFolded(new Set(["x"]), ["y", "z"], true);
    expect([...folded].sort()).toEqual(["x", "y", "z"]);
    expect([...withFolded(folded, ["x", "y"], false)]).toEqual(["z"]);
  });

  it("forget groups that are gone", () => {
    const projects = [{ id: "p1", name: "P", folders: [], created_at: 1 } as unknown as Project];
    const threads = {
      a: thread("a", { project: "p1" } as Partial<Thread>),
      b: thread("b", { cwd: "/other", archived: true }),
    };
    const existing = groupKeys(threads, projects);
    expect([...existing].sort()).toEqual(["folder:/other", "project:p1"]);
    expect([...pruneFolded(new Set(["project:p1", "project:gone", "folder:/other", "folder:/gone"]), existing)]).toEqual([
      "project:p1",
      "folder:/other",
    ]);
  });
});
