import { describe, expect, it } from "vitest";

import type { Thread } from "../api/protocol.gen";
import { isUnread, markSeen, markUnseen, seenEverything } from "./reads";

const thread = (patch: Partial<Thread>) =>
  ({ id: "t", provider: "echo", cwd: "/w", status: "idle", created_at: 1, updated_at: 1, last_seq: 1, ...patch }) as Thread;

describe("unread threads", () => {
  it("are those whose turn ended after the user last looked", () => {
    const done = thread({ last_turn_at: 20 });
    expect(isUnread(done, {})).toBe(true);
    const seen = markSeen({}, done);
    expect(isUnread(done, seen)).toBe(false);
    expect(isUnread(thread({ last_turn_at: 30 }), seen)).toBe(true);
    expect(isUnread(done, markUnseen(seen, done))).toBe(true);
  });

  it("leave out what is running, put away, never ran or belongs to another agent", () => {
    expect(isUnread(thread({ last_turn_at: 20, status: "active" }), {})).toBe(false);
    expect(isUnread(thread({ last_turn_at: 20, archived: true }), {})).toBe(false);
    expect(isUnread(thread({ last_turn_at: 20, parent: "p" }), {})).toBe(false);
    expect(isUnread(thread({}), {})).toBe(false);
  });

  it("start with everything seen", () => {
    const old = thread({ last_turn_at: 20 });
    expect(isUnread(old, seenEverything([old]))).toBe(false);
  });
});
