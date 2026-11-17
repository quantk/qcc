import { describe, expect, it, vi } from "vitest";

import type { Thread } from "../api/protocol.gen";
import {
  ChildFilters,
  childList,
  childState,
  clearNarrowing,
  countChildren,
  DEFAULT_CHILD_FILTER,
  descendantsOf,
  isNarrowed,
  parseViewPrefs,
  type ChildFilter,
} from "./children";

function thread(id: string, created_at: number, parent: string | null, more: Partial<Thread> = {}): Thread {
  return {
    id,
    provider: "echo",
    cwd: "/work",
    parent,
    status: "idle",
    created_at,
    updated_at: created_at,
    last_seq: 1,
    ...more,
  } as Thread;
}

// p started a, b and c; a started a1 and a2; c is archived and started c1.
const family = [
  thread("p", 0, null),
  thread("a", 1, "p", { title: "Review the parser", last_turn_status: "completed", updated_at: 50 }),
  thread("b", 2, "p", { title: "Write tests", status: "active", provider: "codex", updated_at: 30 }),
  thread("c", 3, "p", { title: "Old attempt", archived: true }),
  thread("a1", 4, "a", { title: "Parser fuzzing", last_turn_status: "failed" }),
  thread("a2", 5, "a", { title: "Lexer", pending_interactions: [{ id: "q" }] as Thread["pending_interactions"] }),
  thread("c1", 6, "c", { title: "Leftover" }),
  thread("x", 7, "other"),
  thread("h", 8, "p", { hidden: true }),
];

const filter = (change: Partial<ChildFilter> = {}): ChildFilter => ({ ...DEFAULT_CHILD_FILTER, ...change });
const ids = (list: ReturnType<typeof childList>) => list.rows.map((row) => row.thread.id);

describe("child state", () => {
  it("tells done from idle, and puts waiting first", () => {
    expect(childState(thread("t", 0, null))).toBe("idle");
    expect(childState(thread("t", 0, null, { last_turn_status: "completed" }))).toBe("done");
    expect(childState(thread("t", 0, null, { last_turn_status: "interrupted" }))).toBe("idle");
    expect(childState(thread("t", 0, null, { last_turn_status: "failed" }))).toBe("failed");
    expect(childState(thread("t", 0, null, { status: "failed" }))).toBe("failed");
    expect(childState(thread("t", 0, null, { status: "active" }))).toBe("working");
    expect(
      childState(thread("t", 0, null, { status: "active", pending_interactions: [{}] as Thread["pending_interactions"] })),
    ).toBe("waiting");
  });
});

describe("descendants", () => {
  it("lists parents before their children, without hidden threads", () => {
    expect(descendantsOf(family, "p").map(({ thread, depth }) => `${thread.id}:${depth}`)).toEqual([
      "a:1",
      "a1:2",
      "a2:2",
      "b:1",
      "c:1",
      "c1:2",
    ]);
    expect(descendantsOf(family, "p", 1).map(({ thread }) => thread.id)).toEqual(["a", "b", "c"]);
  });

  it("counts by state, the archived apart", () => {
    const counts = countChildren(family, "p");
    expect(counts.total).toBe(6);
    expect(counts.archived).toBe(1);
    expect(counts.byState).toEqual({ waiting: 1, working: 1, failed: 1, done: 1, idle: 1 });
    expect(countChildren(family, "p", 1).total).toBe(3);
  });

  it("survives a cycle of parents", () => {
    const loop = [thread("u", 0, "v"), thread("v", 1, "u")];
    expect(descendantsOf(loop, "u").length).toBeLessThanOrEqual(40);
  });
});

describe("the agents list", () => {
  it("hides the archived by default, but keeps one as the path to a live child", () => {
    const list = childList(family, "p", filter());
    expect(list.rows.map((row) => [row.thread.id, row.match])).toEqual([
      ["a", true],
      ["a1", true],
      ["a2", true],
      ["b", true],
      ["c", false],
      ["c1", true],
    ]);
    expect(list.shown).toBe(5);
    // Without its live child, an archived thread is simply gone.
    const alone = family.filter((t) => t.id !== "c1");
    expect(ids(childList(alone, "p", filter()))).toEqual(["a", "a1", "a2", "b"]);
  });

  it("shows the archived on request and counts them always", () => {
    const hidden = childList(family, "p", filter());
    const shown = childList(family, "p", filter({ archived: true }));
    expect(hidden.archived).toBe(1);
    expect(shown.archived).toBe(1);
    expect(ids(shown)).toEqual(["a", "a1", "a2", "b", "c", "c1"]);
    expect(shown.rows.map((row) => row.depth)).toEqual([0, 1, 1, 0, 0, 1]);
  });

  it("reaches only the direct children when asked", () => {
    const list = childList(family, "p", filter({ depth: "direct" }));
    expect(ids(list)).toEqual(["a", "b"]);
    expect(list.total).toBe(3);
    expect(list.nested).toBe(true);
    expect(list.rows[0]!.children).toBe(2);
  });

  it("filters by state and keeps the path to a match", () => {
    const list = childList(family, "p", filter({ states: ["waiting"] }));
    expect(list.rows.map((row) => [row.thread.id, row.match])).toEqual([
      ["a", false],
      ["a2", true],
    ]);
    expect(list.shown).toBe(1);
  });

  it("filters by provider and by title", () => {
    expect(ids(childList(family, "p", filter({ providers: ["codex"] })))).toEqual(["b"]);
    expect(ids(childList(family, "p", filter({ query: "  PARSER " })))).toEqual(["a", "a1"]);
    expect(childList(family, "p", filter({ query: "nothing" })).rows).toEqual([]);
  });

  it("counts the options with the archive applied but not the other filters", () => {
    const list = childList(family, "p", filter({ states: ["working"] }));
    expect(list.states).toEqual({ waiting: 1, working: 1, failed: 1, done: 1, idle: 1 });
    expect(list.providers).toEqual(
      new Map([
        ["echo", 4],
        ["codex", 1],
      ]),
    );
  });

  it("sorts siblings, not the tree", () => {
    const recent = ids(childList(family, "p", filter({ sort: "recent", archived: true })));
    expect(recent).toEqual(["a", "a2", "a1", "b", "c", "c1"]);
    const status = ids(childList(family, "p", filter({ sort: "status", archived: true })));
    // b works, c is idle, a is done; under a, the waiting a2 comes before the failed a1.
    expect(status).toEqual(["b", "c", "c1", "a", "a2", "a1"]);
  });

  it("knows when something narrows it", () => {
    expect(isNarrowed(filter())).toBe(false);
    expect(isNarrowed(filter({ archived: true, depth: "direct", sort: "status" }))).toBe(false);
    expect(isNarrowed(filter({ query: " " }))).toBe(false);
    expect(isNarrowed(filter({ query: "x" }))).toBe(true);
    expect(clearNarrowing(filter({ states: ["done"], providers: ["codex"], query: "x", archived: true }))).toEqual(
      filter({ archived: true }),
    );
  });
});

describe("remembered filters", () => {
  it("reads only what it knows from storage", () => {
    expect(parseViewPrefs(null)).toEqual({ archived: false, depth: "all", sort: "started" });
    expect(parseViewPrefs({ archived: true, depth: "direct", sort: "bogus" })).toEqual({
      archived: true,
      depth: "direct",
      sort: "started",
    });
  });

  it("keeps the view for all threads and the narrowing per thread", () => {
    const save = vi.fn();
    const filters = new ChildFilters({ load: () => ({ sort: "status" }), save });
    expect(filters.get("p").sort).toBe("status");
    const before = filters.get("p");
    expect(filters.get("p")).toBe(before);

    const listener = vi.fn();
    filters.subscribe(listener);
    filters.set("p", { query: "parser", states: ["done"] });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(save).not.toHaveBeenCalled();
    expect(filters.get("p").query).toBe("parser");
    expect(filters.get("q").query).toBe("");

    filters.set("q", { archived: true });
    expect(save).toHaveBeenCalledWith({ archived: true, depth: "all", sort: "status" });
    expect(filters.get("p").archived).toBe(true);
    expect(filters.get("p").query).toBe("parser");
  });
});
