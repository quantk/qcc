import { describe, expect, it } from "vitest";

import type { EventPayload, Item, ItemShape, Thread, ThreadEvent } from "../api/protocol.gen";
import { RECENT_MS, contextFill, tailOf, wallThreads } from "./live";
import { applyEvents, emptyTimeline } from "./timeline";

const NOW = 1_000_000_000;

function thread(id: string, patch: Partial<Thread> = {}): Thread {
  return { id, provider: "echo", cwd: "/w", status: "idle", created_at: 0, updated_at: 0, last_seq: 1, ...patch } as Thread;
}

describe("the live wall", () => {
  it("shows who wants you, then who works, then who just finished", () => {
    const shown = wallThreads(
      [
        thread("old", { last_turn_at: NOW - RECENT_MS - 1 }),
        thread("done", { last_turn_at: NOW - 1000 }),
        thread("long", { status: "active", turn_started_at: NOW - 60_000 }),
        thread("short", { status: "active", turn_started_at: NOW - 1000 }),
        thread("asks", {
          status: "active",
          pending_interactions: [{ id: "i", request: { type: "question", question: "?" } }],
        } as Partial<Thread>),
        thread("put-away", { status: "active", archived: true }),
      ],
      NOW,
    );
    expect(shown.map((t) => t.id)).toEqual(["asks", "long", "short", "done"]);
  });

  it("tells how full the context is only when the agent said", () => {
    expect(contextFill(thread("a"))).toBeNull();
    const usage = { context_used: 50, context_window: 200 } as Thread["usage"];
    expect(contextFill(thread("a", { usage }))).toBe(0.25);
  });

  it("keeps the last steps, with the end of what they printed", () => {
    const item = (shape: ItemShape, id: string, status: Item["status"] = "completed"): EventPayload => ({
      type: "item/completed",
      turn_id: "u1",
      item: { id, status, ...shape } as Item,
    });
    const events: ThreadEvent[] = [
      { type: "turn/started", turn_id: "u1" } as EventPayload,
      item({ type: "reasoning", text: "hm" }, "r"),
      item({ type: "fileChange", path: "/w/a.rs" }, "e"),
      item({ type: "command", command: "make", output: "1\n2\n3\n4\n", exit_code: 2 }, "c"),
      item({ type: "agentMessage", text: "done" }, "m"),
    ].map((payload, index) => ({ thread_id: "t", seq: index + 1, ts: index, ...payload }));
    const { entries } = applyEvents(emptyTimeline, events);
    const tail = tailOf(entries, 2);
    expect(tail.map((line) => line.kind)).toEqual(["command", "message"]);
    expect(tail[0]).toMatchObject({ text: "make", detail: "2\n3\n4", failed: true });
    // A long line keeps its end.
    const long = applyEvents(emptyTimeline, [
      { thread_id: "t", seq: 1, ts: 0, ...item({ type: "agentMessage", text: "x".repeat(500) + " the end" }, "m") },
    ]);
    expect(tailOf(long.entries, 1)[0]!.detail).toMatch(/^….*x the end$/);
    // A settled thought says nothing worth a line.
    expect(tailOf(entries, 10).map((line) => line.kind)).toEqual(["edit", "command", "message"]);
  });
});
