import { describe, expect, it } from "vitest";

import { estimateEntryHeight, newTurnKey, turnRoom } from "./scroll";
import type { Entry } from "./timeline";

const user = (key: string, text = "hi"): Entry => ({ kind: "user", key, ts: 0, text, notes: [], steered: false, queued: false });
const answer = (key: string, text: string): Entry => ({
  kind: "item",
  key,
  ts: 0,
  item: { id: key, status: "completed", type: "agentMessage", text },
});

describe("scroll", () => {
  it("guesses taller entries for longer text", () => {
    const short = estimateEntryHeight(answer("1", "ok"));
    const long = estimateEntryHeight(answer("2", "word ".repeat(200)));
    const paragraphs = estimateEntryHeight(answer("3", "a\n\nb\n\nc"));
    expect(long).toBeGreaterThan(short);
    expect(paragraphs).toBeGreaterThan(short);
    expect(estimateEntryHeight({ kind: "conversation", key: "c", ts: 0 })).toBeGreaterThan(0);
  });

  it("finds the user message that starts a new turn", () => {
    const before = [user("1"), answer("2", "done")];
    expect(newTurnKey(before, [...before, user("3")])).toBe("3");
    expect(newTurnKey(before, [...before, user("3"), answer("4", "on it")])).toBe("3");
    // The answer growing, or more of it, is not a new turn.
    expect(newTurnKey(before, [user("1"), answer("2", "done!")])).toBeNull();
    expect(newTurnKey(before, [...before, answer("4", "more")])).toBeNull();
    // Opening a thread, or showing its earlier conversations, brings no new turn.
    expect(newTurnKey([], before)).toBe(null);
    expect(newTurnKey(before, [user("0"), ...before])).toBeNull();
  });

  it("makes room for a new turn until its answer fills the view", () => {
    const room = (contentEnd: number) => turnRoom({ anchorStart: 1000, peek: 64, contentEnd, viewportHeight: 600 });
    // The message alone: room for it to rise to the top, the entry above peeking in.
    expect(room(1040)).toBe(1000 - 64 + 600 - 1040);
    // The answer grows into the room...
    expect(room(1300)).toBe(1000 - 64 + 600 - 1300);
    // ...and once it fills the view, there is none.
    expect(room(1600)).toBe(0);
    expect(turnRoom({ anchorStart: 10, peek: 64, contentEnd: 100, viewportHeight: 600 })).toBe(500);
  });
});
