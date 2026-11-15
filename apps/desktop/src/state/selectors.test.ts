import { describe, expect, it } from "vitest";

import type { Thread } from "../api/protocol.gen";
import { childrenOf, rootOf, threadTree } from "./selectors";

function thread(id: string, created_at: number, parent?: string): Thread {
  return {
    id,
    provider: "echo",
    cwd: "/work",
    parent: parent ?? null,
    status: "idle",
    created_at,
    updated_at: created_at,
    last_seq: 1,
  } as Thread;
}

describe("thread families", () => {
  const threads = [
    thread("a", 1),
    thread("b", 5),
    thread("a1", 2, "a"),
    thread("a2", 3, "a"),
    thread("a1x", 4, "a1"),
    thread("orphan", 6, "gone"),
  ];

  it("puts children under their parent, in the order they were started", () => {
    const rows = threadTree(threads).map((row) => `${"-".repeat(row.depth)}${row.thread.id}:${row.children}`);
    expect(rows).toEqual(["orphan:0", "b:0", "a:2", "-a1:1", "--a1x:0", "-a2:0"]);
  });

  it("finds the root of a family and the children of a thread", () => {
    const byId: Record<string, Thread> = Object.fromEntries(threads.map((t) => [t.id, t]));
    const find = (id: string) => byId[id] as Thread;
    expect(rootOf(find("a1x"), byId).id).toBe("a");
    expect(rootOf(find("orphan"), byId).id).toBe("orphan");
    expect(childrenOf(threads, "a").map((t) => t.id)).toEqual(["a1", "a2"]);
  });
});
