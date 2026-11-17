import { describe, expect, it } from "vitest";

import type { Thread } from "../api/protocol.gen";
import { tallyChildren, tallyLabel, tallyParts } from "./lineage";

function thread(id: string, patch: Partial<Thread> = {}): Thread {
  return { id, provider: "echo", cwd: "/w", status: "idle", created_at: 0, updated_at: 0, last_seq: 1, ...patch } as Thread;
}

const asks = { pending_interactions: [{ id: "i", request: { type: "question", question: "?" } }] } as Partial<Thread>;

describe("the children of a thread at a glance", () => {
  it("puts every child in one count, a question before work", () => {
    const tally = tallyChildren([
      thread("works", { status: "active" }),
      thread("asks", { status: "active", ...asks }),
      thread("broke", { status: "failed" }),
      thread("turn-failed", { last_turn_status: "failed" }),
      thread("finished", { last_turn_status: "completed" }),
      thread("stopped", { last_turn_status: "interrupted" }),
      thread("put-away", { status: "active", archived: true }),
    ]);
    expect(tally).toEqual({ total: 7, waiting: 1, working: 1, failed: 2, done: 3 });
  });

  it("counts nothing when there are no children", () => {
    expect(tallyChildren([])).toEqual({ total: 0, waiting: 0, working: 0, failed: 0, done: 0 });
    expect(tallyParts(tallyChildren([]))).toEqual([]);
  });

  it("names only the counts that are not zero, the urgent ones first", () => {
    const tally = tallyChildren([thread("a"), thread("b", { status: "active" }), thread("c", { status: "active", ...asks })]);
    expect(tallyParts(tally).map((part) => part.key)).toEqual(["waiting", "working", "done"]);
    expect(tallyLabel(tally)).toBe("3 agents started here: 1 waiting for you, 1 working, 1 done");
    expect(tallyLabel(tallyChildren([thread("a", { status: "active" })]))).toBe("1 agent started here: 1 working");
  });
});
