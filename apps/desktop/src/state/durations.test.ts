import { describe, expect, it } from "vitest";

import type { EventPayload, ThreadEvent } from "../api/protocol.gen";
import { durationLabel, timingHint, turnTimings, untilShown } from "./durations";
import { applyEvents, emptyTimeline } from "./timeline";

describe("durationLabel", () => {
  it("names a settled action of two seconds or more", () => {
    expect(durationLabel({ startedAt: 1000, endedAt: 48_000 }, false, 0)).toBe("47s");
    expect(durationLabel({ startedAt: 0, endedAt: 192_000 }, false, 0)).toBe("3m 12s");
    expect(durationLabel({ startedAt: 0, endedAt: 2000 }, false, 0)).toBe("2s");
  });

  it("says nothing of a quick one, or of one started and settled at once", () => {
    expect(durationLabel({ startedAt: 0, endedAt: 1999 }, false, 0)).toBeNull();
    expect(durationLabel({ startedAt: 5000, endedAt: 5000 }, false, 0)).toBeNull();
  });

  it("says nothing without timing, or of a settled one with no end", () => {
    expect(durationLabel(undefined, false, 0)).toBeNull();
    expect(durationLabel(undefined, true, 10_000)).toBeNull();
    expect(durationLabel({ startedAt: 0 }, false, 10_000)).toBeNull();
  });

  it("counts a running action once it has gone on five seconds", () => {
    expect(durationLabel({ startedAt: 0 }, true, 4999)).toBeNull();
    expect(durationLabel({ startedAt: 0 }, true, 5000)).toBe("5s");
    expect(durationLabel({ startedAt: 0 }, true, 65_000)).toBe("1m 05s");
  });
});

describe("untilShown", () => {
  it("waits for the rest of the five seconds, then not at all", () => {
    expect(untilShown({ startedAt: 1000 }, 2000)).toBe(4000);
    expect(untilShown({ startedAt: 1000 }, 9000)).toBe(0);
  });
});

describe("timingHint", () => {
  it("gives the start, and the end once there is one", () => {
    const start = new Date(2026, 10, 23, 9, 5, 7).getTime();
    expect(timingHint({ startedAt: start })).toMatch(/^Started .+$/);
    expect(timingHint({ startedAt: start })).not.toContain("ended");
    const hint = timingHint({ startedAt: start, endedAt: start + 47_000 });
    expect(hint).toMatch(/^Started .+, ended .+$/);
  });

  it("names the day of the end only when it differs", () => {
    const start = new Date(2026, 10, 23, 23, 59, 0).getTime();
    const sameDay = timingHint({ startedAt: start, endedAt: start + 30_000 });
    const nextDay = timingHint({ startedAt: start, endedAt: start + 120_000 });
    expect(nextDay.split(", ended ")[1]!.length).toBeGreaterThan(sameDay.split(", ended ")[1]!.length);
  });
});

describe("turnTimings", () => {
  function fold(...payloads: [number, EventPayload][]) {
    const events: ThreadEvent[] = payloads.map(([ts, payload], index) => ({
      thread_id: "t",
      seq: index + 1,
      ts,
      ...payload,
    }));
    return applyEvents(emptyTimeline, events);
  }
  const reply = (turn: string, id: string): EventPayload => ({
    type: "item/completed",
    turn_id: turn,
    item: { id, status: "completed", type: "agentMessage", text: "re" },
  });

  it("puts each ended turn's time under its last reply", () => {
    const timeline = fold(
      [0, { type: "turn/started", turn_id: "u1" }],
      [10_000, reply("u1", "a")],
      [20_000, reply("u1", "b")],
      [21_000, { type: "turn/completed", turn_id: "u1", status: "completed" }],
      [30_000, { type: "turn/started", turn_id: "u2" }],
      [40_000, reply("u2", "c")],
    );
    const timings = turnTimings(timeline.turns, timeline.stats.turns);
    expect(timings.get("b")).toEqual({ startedAt: 0, endedAt: 21_000 });
    // An earlier reply of the turn, and the reply of a turn still running, get none.
    expect(timings.has("a")).toBe(false);
    expect(timings.has("c")).toBe(false);
  });

  it("skips a turn the journal saw end without starting", () => {
    const timeline = fold([10_000, reply("u1", "a")], [21_000, { type: "turn/completed", turn_id: "u1", status: "completed" }]);
    expect(turnTimings(timeline.turns, timeline.stats.turns).size).toBe(0);
  });
});
