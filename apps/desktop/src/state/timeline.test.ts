import { describe, expect, it } from "vitest";

import type { EventPayload, Item, ThreadEvent } from "../api/protocol.gen";
import { type Timeline, applyEvents, emptyTimeline, pendingInteractions, queuedMessages } from "./timeline";

function events(...payloads: EventPayload[]): ThreadEvent[] {
  return payloads.map((payload, index) => ({ thread_id: "t", seq: index + 1, ts: 1000 + index, ...payload }));
}

const fold = (...payloads: EventPayload[]): Timeline => applyEvents(emptyTimeline, events(...payloads));

const message = (text: string, status: Item["status"] = "pending"): Item => ({
  id: "i1",
  status,
  type: "agentMessage",
  text,
});

describe("timeline", () => {
  it("streams text into an item and settles it with the final shape", () => {
    const timeline = fold(
      { type: "input/submitted", text: "hi" },
      { type: "turn/started", turn_id: "u1" },
      { type: "item/started", turn_id: "u1", item: message("") },
      { type: "item/text/delta", item_id: "i1", text: "Hel" },
      { type: "item/text/delta", item_id: "i1", text: "lo" },
    );
    expect(timeline.turnOpen).toBe(true);
    expect(timeline.entries.map((entry) => entry.kind)).toEqual(["user", "item"]);
    expect(timeline.entries[1]).toMatchObject({ item: { text: "Hello", status: "pending" } });

    const settled = applyEvents(timeline, [
      { thread_id: "t", seq: 6, ts: 2000, type: "item/completed", turn_id: "u1", item: message("Hello!", "completed") },
      { thread_id: "t", seq: 7, ts: 2001, type: "turn/completed", turn_id: "u1", status: "completed" },
    ]);
    expect(settled.turnOpen).toBe(false);
    expect(settled.entries).toHaveLength(2);
    expect(settled.entries[1]).toMatchObject({ item: { text: "Hello!", status: "completed" } });
  });

  it("ignores events it has already seen", () => {
    const all = events({ type: "input/submitted", text: "one" }, { type: "input/submitted", text: "two" });
    const once = applyEvents(emptyTimeline, all);
    const twice = applyEvents(once, all);
    expect(twice).toBe(once);
    expect(twice.entries).toHaveLength(2);
  });

  it("moves a queued message to where it was delivered", () => {
    const timeline = fold(
      { type: "input/submitted", text: "first" },
      { type: "input/queued", queue_id: "q1", text: "second" },
      { type: "input/queued", queue_id: "q2", text: "third" },
      { type: "item/started", turn_id: "u1", item: message("answer", "completed") },
      { type: "input/submitted", text: "second", queue_id: "q1" },
      { type: "input/dropped", queue_id: "q2" },
    );
    expect(timeline.entries.map((entry) => (entry.kind === "user" ? entry.text : entry.kind))).toEqual([
      "first",
      "item",
      "second",
    ]);
    expect(timeline.entries.every((entry) => entry.kind !== "user" || !entry.queued)).toBe(true);
  });

  it("tracks interactions until they are resolved", () => {
    const request = { type: "approval" as const, title: "Run ls", options: [] };
    const asked = fold({ type: "interaction/requested", interaction_id: "x1", request });
    expect(pendingInteractions(asked)).toHaveLength(1);
    const answered = applyEvents(asked, [
      {
        thread_id: "t",
        seq: 2,
        ts: 1,
        type: "interaction/resolved",
        interaction_id: "x1",
        resolution: { type: "selected", option_id: "allow" },
      },
    ]);
    expect(pendingInteractions(answered)).toHaveLength(0);
    expect(answered.entries[0]).toMatchObject({ resolution: { type: "selected", option_id: "allow" } });
  });

  it("reports failures and interruptions as notices", () => {
    const timeline = fold(
      { type: "turn/started", turn_id: "u1" },
      { type: "turn/completed", turn_id: "u1", status: "failed", error: "boom" },
      { type: "turn/started", turn_id: "u2" },
      { type: "turn/completed", turn_id: "u2", status: "interrupted" },
      { type: "environment/failed", message: "not a git repository" },
    );
    expect(timeline.entries).toMatchObject([
      { kind: "notice", level: "error", text: "boom" },
      { kind: "notice", level: "info", text: "Interrupted" },
      { kind: "notice", level: "error", text: "Environment failed: not a git repository" },
    ]);
  });
});

describe("queued messages", () => {
  it("are listed until delivered or dropped", () => {
    const event = (seq: number, payload: object) => ({ thread_id: "t", seq, ts: seq, ...payload }) as ThreadEvent;
    let timeline = applyEvents(emptyTimeline, [
      event(1, { type: "input/queued", queue_id: "a", text: "first" }),
      event(2, { type: "input/queued", queue_id: "b", text: "second" }),
      event(3, { type: "input/queued", queue_id: "c", text: "third" }),
    ]);
    expect(queuedMessages(timeline).map((m) => m.queueId)).toEqual(["a", "b", "c"]);
    timeline = applyEvents(timeline, [
      event(4, { type: "input/submitted", text: "first", queue_id: "a" }),
      event(5, { type: "input/dropped", queue_id: "c" }),
    ]);
    expect(queuedMessages(timeline)).toEqual([{ queueId: "b", text: "second" }]);
    expect(queuedMessages(undefined)).toEqual([]);
  });
});
