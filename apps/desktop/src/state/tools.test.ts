import { describe, expect, it } from "vitest";

import type { EventPayload, Item, ItemShape, ThreadEvent, ToolBucket } from "../api/protocol.gen";
import { applyEvents, emptyTimeline } from "./timeline";
import { byServer, ofKind, program, rowLabel, toolsOfBuckets, toolsOfTimeline } from "./tools";

const tool = (name: string, extra: { mcp_server?: string; skill?: string } = {}): ItemShape =>
  ({ type: "tool", name, input: {}, ...extra }) as ItemShape;

/** Each item starts at `ts` and ends `took` later. */
function timeline(...items: { shape: ItemShape; status?: Item["status"]; ts: number; took: number }[]) {
  const events: ThreadEvent[] = [];
  items.forEach(({ shape, status = "completed", ts, took }, index) => {
    const item = { id: `i${index}`, ...shape } as Item;
    const payloads: [number, EventPayload][] = [
      [ts, { type: "item/started", turn_id: "u", item: { ...item, status: "pending" } }],
      [ts + took, { type: "item/completed", turn_id: "u", item: { ...item, status } }],
    ];
    for (const [at, payload] of payloads) events.push({ thread_id: "t", seq: events.length + 1, ts: at, ...payload });
  });
  return applyEvents(emptyTimeline, events).entries;
}

const bucket = (thread: string, kind: ToolBucket["kind"], name: string, calls: number, extra: Partial<ToolBucket> = {}): ToolBucket => ({
  thread_id: thread,
  hour: 0,
  kind,
  name,
  calls,
  failed: 0,
  declined: 0,
  interrupted: 0,
  duration_ms: 0,
  longest_ms: 0,
  ...extra,
});

describe("tools", () => {
  it("counts a thread's uses, how they ended and how long they took", () => {
    const command = (exit_code: number): ItemShape => ({ type: "command", command: "cargo test", exit_code }) as ItemShape;
    const rows = toolsOfTimeline(
      timeline(
        { shape: command(0), ts: 0, took: 100 },
        { shape: command(1), ts: 200, took: 300 },
        { shape: tool("mcp__docs__search", { mcp_server: "docs" }), status: "failed", ts: 600, took: 50 },
        { shape: tool("Skill", { skill: "tdd" }), ts: 700, took: 10 },
      ),
    );
    expect(rows.map((row) => [row.kind, row.name, row.calls, row.failed])).toEqual([
      ["command", "cargo", 2, 1],
      ["mcp", "mcp__docs__search", 1, 1],
      ["skill", "tdd", 1, 0],
    ]);
    expect([rows[0]!.durationMs, rows[0]!.longestMs]).toEqual([400, 300]);
    expect(rowLabel(rows[1]!)).toBe("docs · search");
  });

  it("adds up buckets over the threads it keeps", () => {
    const buckets = [
      bucket("a", "tool", "TodoWrite", 2, { longest_ms: 5 }),
      bucket("b", "tool", "TodoWrite", 3, { longest_ms: 9 }),
      bucket("b", "mcp", "docs.search", 4, { server: "docs", failed: 1 }),
      bucket("c", "mcp", "web.fetch", 1, { server: "web" }),
    ];
    const all = toolsOfBuckets(buckets);
    expect(all.map((row) => [row.name, row.calls, row.threads])).toEqual([
      ["TodoWrite", 5, 2],
      ["docs.search", 4, 1],
      ["web.fetch", 1, 1],
    ]);
    expect(all[0]!.longestMs).toBe(9);
    expect(byServer(all)).toEqual([
      { server: "docs", calls: 4, failed: 1, tools: 1 },
      { server: "web", calls: 1, failed: 0, tools: 1 },
    ]);
    expect(toolsOfBuckets(buckets, (id) => id === "a").map((row) => row.calls)).toEqual([2]);
    expect(ofKind(all, "mcp").map((row) => rowLabel(row))).toEqual(["docs · search", "web · fetch"]);
  });

  it("reads a command's program as the daemon does", () => {
    expect(program("RUST_LOG=debug /usr/bin/cargo test")).toBe("cargo");
    expect(program("  git status")).toBe("git");
  });
});
