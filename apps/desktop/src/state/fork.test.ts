import { describe, expect, it } from "vitest";

import type { EventPayload, ForkSupport, Thread, ThreadEvent } from "../api/protocol.gen";
import { type Agent, applyTurns, emptyTurns, forkBlocker, turnOfReply } from "./fork";

function fold(...payloads: EventPayload[]) {
  const events: ThreadEvent[] = payloads.map((payload, index) => ({
    thread_id: "t",
    seq: index + 1,
    ts: index,
    ...payload,
  }));
  return events.reduce(applyTurns, emptyTurns);
}

const created: EventPayload = {
  type: "thread/created",
  thread: { id: "t", provider: "echo", cwd: "/w", status: "idle", created_at: 0, updated_at: 0, last_seq: 0 } as Thread,
};

/** A turn that ends at `point`, its reply item `r<n>`. */
function turn(n: number, point?: string): EventPayload[] {
  return [
    { type: "input/submitted", text: `prompt ${n}` },
    { type: "turn/started", turn_id: `u${n}` },
    { type: "item/completed", turn_id: `u${n}`, item: { id: `r${n}`, status: "completed", type: "agentMessage", text: "re" } },
    { type: "turn/completed", turn_id: `u${n}`, status: "completed", point },
  ];
}

const agent = (fork: ForkSupport): Agent => ({ name: "Echo", capabilities: { resume: true, steer: false, fork } });

describe("fork points", () => {
  it("are offered under each ended turn's last reply, in its session", () => {
    const turns = fold(
      created,
      { type: "session/started", provider_session: "s1", resumed: false },
      ...turn(1, "p1"),
      ...turn(2, "p2"),
    );
    expect(turnOfReply(turns, "r1")).toMatchObject({ turnId: "u1", point: "p1", session: "s1", provider: "echo" });
    expect(turnOfReply(turns, "r2")?.turnId).toBe("u2");
    expect(turnOfReply(turns, "elsewhere")).toBeUndefined();
    expect(forkBlocker(turns, turnOfReply(turns, "r1")!, agent("any"))).toBeNull();
  });

  it("say why a reply cannot be forked from", () => {
    const turns = fold(
      created,
      { type: "session/started", provider_session: "s1", resumed: false },
      ...turn(1, "p1"),
      ...turn(2, "p2"),
      ...turn(3),
    );
    const [first, second, third] = turns.ended;
    expect(forkBlocker(turns, third!, agent("any"))).toMatch(/no point to fork from/);
    expect(forkBlocker(turns, first!, agent("latest"))).toMatch(/only from its latest reply/);
    // The second turn is not its session's latest either: the third ran in it.
    expect(forkBlocker(turns, second!, agent("latest"))).toMatch(/latest/);
    expect(forkBlocker(turns, first!, agent("none"))).toBe("Echo cannot fork a conversation");
    expect(forkBlocker(turns, first!, undefined)).toBe("No plugin provides echo now");
  });

  it("follow the session: a new conversation or another agent leaves the old turns their own", () => {
    const turns = fold(
      created,
      { type: "session/started", provider_session: "s1", resumed: false },
      ...turn(1, "p1"),
      { type: "thread/reset" },
      { type: "session/started", provider_session: "s2", resumed: false },
      ...turn(2, "p2"),
      { type: "thread/agent", provider: "other" },
      ...turn(3, "p3"),
    );
    expect(turns.ended.map((ended) => [ended.session, ended.provider])).toEqual([
      ["s1", "echo"],
      ["s2", "echo"],
      [undefined, "other"],
    ]);
    // The latest of its own session, whatever came after in others.
    expect(forkBlocker(turns, turns.ended[0]!, agent("latest"))).toBeNull();
  });

  it("count a turn running now as the session's latest", () => {
    const turns = fold(created, { type: "session/started", provider_session: "s1", resumed: false }, ...turn(1, "p1"));
    expect(forkBlocker(turns, turns.ended[0]!, agent("latest"))).toBeNull();
    const running = applyTurns(turns, { thread_id: "t", seq: 99, ts: 0, type: "turn/started", turn_id: "u9" });
    expect(forkBlocker(running, running.ended[0]!, agent("latest"))).toMatch(/latest/);
  });

  it("start a fork in the session it was forked from until the agent names its own", () => {
    const forked = {
      ...created,
      thread: { ...(created as { thread: Thread }).thread, provider_session: "source" },
    } as EventPayload;
    const turns = fold(forked, { type: "session/identified", provider_session: "mine" }, ...turn(1, "p1"));
    expect(turns.ended[0]?.session).toBe("mine");
  });
});
