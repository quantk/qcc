import { describe, expect, it } from "vitest";

import type { Project, Thread, UsageBucket } from "../api/protocol.gen";
import {
  UNKNOWN,
  averageTurn,
  byDay,
  byGroup,
  dayStart,
  failureRate,
  groupOf,
  inRange,
  seriesSlots,
  tokensOf,
  topThreads,
} from "./metrics";

const HOUR = 3_600_000;
const NOW = new Date(2026, 10, 16, 15, 0).getTime();
const daysAgo = (days: number, hour = 10) => new Date(2026, 10, 16 - days, hour).getTime();

function bucket(thread_id: string, hour: number, input: number, extra: Partial<UsageBucket> = {}): UsageBucket {
  return {
    thread_id,
    hour,
    spent: { input_tokens: input, output_tokens: 1, cached_input_tokens: 5 },
    turns_completed: 1,
    turns_failed: 0,
    turns_interrupted: 0,
    working_ms: 1000,
    ...extra,
  };
}

function thread(id: string, extra: Partial<Thread> = {}): Thread {
  return { id, provider: "codex", cwd: `/src/${id}`, status: "idle", created_at: 0, updated_at: 0, last_seq: 1, ...extra } as Thread;
}

const threads = {
  a: thread("a", { project: "p", model: "gpt-x" }),
  b: thread("b", { provider: "claude-code" }),
};
const projects = [{ id: "p", name: "App", folders: [], created_at: 0 }] as Project[];
const known = { threads, projects, plugins: [] };

describe("metrics", () => {
  it("keep the buckets of the range, counted in local days", () => {
    const buckets = [bucket("a", daysAgo(10), 1), bucket("a", daysAgo(6), 2), bucket("a", daysAgo(0), 3)];
    expect(inRange(buckets, 7, NOW).map((b) => b.spent.input_tokens)).toEqual([2, 3]);
    expect(inRange(buckets, "all", NOW)).toHaveLength(3);
  });

  it("group spending by project, agent and model, deleted threads apart", () => {
    const buckets = [bucket("a", daysAgo(1), 10), bucket("b", daysAgo(1), 30), bucket("gone", daysAgo(1), 1)];
    const project = byGroup(buckets, groupOf("project", known));
    expect(project.map((row) => row.label)).toEqual(["b", "App", UNKNOWN.label]);
    expect(byGroup(buckets, groupOf("model", known)).map((row) => row.label)).toEqual([
      "agent default",
      "gpt-x",
      UNKNOWN.label,
    ]);
    expect(byGroup(buckets, groupOf("agent", known)).map((row) => row.key)).toEqual(["claude-code", "codex", "unknown"]);
    // Cached input is shown apart and left out of the measure.
    expect(project[0]!.totals).toMatchObject({ input: 30, cached: 5, output: 1 });
    expect(tokensOf(project[0]!.totals)).toBe(31);
  });

  it("rate failures and time turns among those that ended", () => {
    const [row] = byGroup(
      [bucket("a", 0, 1), bucket("a", HOUR, 1, { turns_completed: 0, turns_failed: 1, working_ms: 3000 })],
      groupOf("agent", known),
    );
    expect(failureRate(row!.totals)).toBe(0.5);
    expect(averageTurn(row!.totals)).toBe(2000);
    expect(failureRate(byGroup([bucket("a", 0, 1, { turns_completed: 0 })], groupOf("agent", known))[0]!.totals)).toBe(
      null,
    );
  });

  it("lay out every day of the range, empty ones too", () => {
    const days = byDay([bucket("a", daysAgo(2, 9), 4), bucket("a", daysAgo(2, 20), 6)], groupOf("project", known), 7, NOW);
    expect(days).toHaveLength(7);
    expect(days[0]!.day).toBe(dayStart(daysAgo(6)));
    expect(days.at(-1)!.day).toBe(dayStart(NOW));
    const busy = days.find((day) => day.day === dayStart(daysAgo(2)))!;
    expect(busy.groups.get("project:p")!.input).toBe(10);
    expect(days.filter((day) => day.groups.size > 0)).toHaveLength(1);
  });

  it("give colors by all-time spending and list the threads that spent most", () => {
    const buckets = [bucket("a", 0, 5), bucket("b", 0, 50), bucket("c", 0, 0, { spent: { input_tokens: 0, output_tokens: 0 } })];
    const slots = seriesSlots(byGroup(buckets, groupOf("agent", known)));
    expect([...slots]).toEqual([
      ["claude-code", 0],
      ["codex", 1],
      ["unknown", 2],
    ]);
    expect(topThreads(buckets, 5).map((entry) => entry.threadId)).toEqual(["b", "a"]);
  });
});
