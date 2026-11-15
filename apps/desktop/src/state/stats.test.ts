import { describe, expect, it } from "vitest";

import type { EventPayload, Item, ItemShape, ThreadEvent, TokenUsage } from "../api/protocol.gen";
import { activityOf, contextUsed, threadTotal, turnSummary } from "./stats";
import { applyEvents, emptyTimeline } from "./timeline";

function fold(...payloads: EventPayload[]) {
  const events: ThreadEvent[] = payloads.map((payload, index) => ({
    thread_id: "t",
    seq: index + 1,
    ts: 1000 * (index + 1),
    ...payload,
  }));
  return applyEvents(emptyTimeline, events);
}

const tokens = (input: number, output: number, cached?: number): TokenUsage => ({
  input_tokens: input,
  output_tokens: output,
  cached_input_tokens: cached ?? null,
});

const usage = (total: TokenUsage, last: TokenUsage = total): EventPayload => ({
  type: "usage/updated",
  usage: { total, last, model_context_window: 1000 },
});

const item = (shape: ItemShape, id: string, status: Item["status"] = "completed"): EventPayload => ({
  type: "item/completed",
  turn_id: "u1",
  item: { id, status, ...shape } as Item,
});

describe("stats", () => {
  it("times turns and tells the running one apart", () => {
    const { stats } = fold(
      { type: "turn/started", turn_id: "u1" },
      { type: "turn/completed", turn_id: "u1", status: "completed" },
      { type: "turn/started", turn_id: "u2" },
      { type: "turn/completed", turn_id: "u2", status: "failed" },
      { type: "turn/started", turn_id: "u3" },
    );
    expect(turnSummary(stats.turns)).toEqual({
      count: 3,
      completed: 1,
      failed: 1,
      interrupted: 0,
      working: 2000,
      longest: 1000,
      running: 5000,
    });
  });

  it("adds up what every session spent, though each counts from zero", () => {
    const { stats } = fold(
      usage(tokens(100, 10, 50)),
      usage(tokens(300, 30, 150)),
      // A new session after a restart: the count went down.
      usage(tokens(40, 4, 0)),
      { type: "thread/reset" },
      usage(tokens(5, 1)),
    );
    expect(threadTotal(stats)).toEqual(tokens(345, 35, 150));
    // The chart and compactions are of the current conversation only.
    expect(stats.usage).toHaveLength(1);
  });

  it("does not count twice a total that carries over a resume", () => {
    const { stats } = fold(usage(tokens(100, 10)), { type: "session/started", resumed: true }, usage(tokens(150, 20)));
    expect(threadTotal(stats)).toEqual(tokens(150, 20));
  });

  it("measures the context by the last request, cache included", () => {
    expect(contextUsed({ total: tokens(0, 0), last: tokens(20, 5, 300) })).toBe(320);
  });

  it("counts what the agent did", () => {
    const { entries } = fold(
      { type: "input/submitted", text: "go" },
      item({ type: "command", command: "ls", exit_code: 0 }, "c1"),
      item({ type: "command", command: "false", exit_code: 1 }, "c2"),
      item({ type: "fileChange", path: "/w/a.rs" }, "f1"),
      item({ type: "fileChange", path: "/w/a.rs" }, "f2"),
      item({ type: "fileRead", path: "/w/b.rs" }, "r1"),
      item({ type: "tool", name: "grep" }, "t1"),
      item({ type: "tool", name: "web" }, "t2"),
      item({ type: "tool", name: "web" }, "t3"),
      item({ type: "agentMessage", text: "done" }, "m1"),
    );
    expect(activityOf(entries)).toMatchObject({
      userMessages: 1,
      agentMessages: 1,
      commands: 2,
      failedCommands: 1,
      filesChanged: ["/w/a.rs"],
      filesRead: ["/w/b.rs"],
      tools: [
        { name: "web", count: 2 },
        { name: "grep", count: 1 },
      ],
    });
  });
});
