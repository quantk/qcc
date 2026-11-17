import { describe, expect, it } from "vitest";

import type { ContextUsage, Thread, UsageSnapshot } from "../api/protocol.gen";
import { contextOf, estimate, groupOf } from "./context";
import { emptyStats } from "./stats";

const thread = (extra: Partial<Thread> = {}): Thread =>
  ({ id: "t", provider: "echo", cwd: "/w", status: "idle", created_at: 0, updated_at: 0, last_seq: 0, ...extra }) as Thread;

const snapshot = (input: number, cached = 0): UsageSnapshot => ({
  total: { input_tokens: input, output_tokens: 0, cached_input_tokens: cached },
  last: { input_tokens: input, output_tokens: 0, cached_input_tokens: cached },
  model_context_window: 1000,
});

const said: ContextUsage = {
  used: 300,
  window: 1000,
  categories: [
    { id: "system_prompt", name: "System prompt", tokens: 100, kind: "used" },
    { id: "skills", name: "Skills", tokens: 50, kind: "used" },
    { id: "messages", name: "Messages", tokens: 150, kind: "used" },
    { id: "mcp_tools_deferred", name: "MCP tools (deferred)", tokens: 70, kind: "deferred" },
    { id: "autocompact_buffer", name: "Autocompact buffer", tokens: 100, kind: "buffer" },
    { id: "free_space", name: "Free space", tokens: 600, kind: "free" },
  ],
  entries: [
    { category: "skills", name: "tdd", tokens: 30 },
    { category: "skills", name: "docs", tokens: 20 },
  ],
};

describe("context", () => {
  it("draws the agent's own breakdown, its entries under their parts", () => {
    const view = contextOf(thread(), { ...emptyStats, context: said })!;
    expect(view.estimated).toBe(false);
    expect(view.usedParts.map((part) => [part.id, part.group])).toEqual([
      ["system_prompt", "system"],
      ["skills", "memory"],
      ["messages", "messages"],
    ]);
    expect(view.usedParts[1]!.entries.map((entry) => entry.name)).toEqual(["tdd", "docs"]);
    expect([view.buffer, view.free]).toEqual([100, 600]);
    expect(view.deferred.map((part) => part.id)).toEqual(["mcp_tools_deferred"]);
  });

  it("falls back to the projection while the timeline loads", () => {
    const view = contextOf(thread({ context: { ...said, entries: [] } }), undefined)!;
    expect(view.used).toBe(300);
    expect(view.usedParts[1]!.entries).toEqual([]);
  });

  it("reckons the start of the conversation from its smallest request", () => {
    const points = [400, 200, 650].map((input, index) => ({ ts: index, usage: snapshot(input) }));
    const reckoned = estimate(points, null)!;
    expect(reckoned.estimated).toBe(true);
    expect(reckoned.window).toBe(1000);
    expect(reckoned.categories.map((category) => [category.id, category.tokens])).toEqual([
      ["start", 200],
      ["messages", 450],
    ]);
    const view = contextOf(thread(), { ...emptyStats, usage: points })!;
    expect([view.used, view.free]).toEqual([650, 350]);
  });

  it("knows only how full it is from the projection alone", () => {
    const view = contextOf(thread({ usage: { context_used: 40, context_window: 100 } as Thread["usage"] }), undefined)!;
    expect([view.used, view.window, view.free, view.estimated]).toEqual([40, 100, 60, true]);
    expect(contextOf(thread(), undefined)).toBeNull();
  });

  it("keeps a part's color whatever else is there", () => {
    expect(groupOf("mcp_tools")).toBe("mcp");
    expect(groupOf("something_new")).toBe("other");
  });
});
