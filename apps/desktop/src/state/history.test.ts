import { describe, expect, it } from "vitest";

import { startHistory, step, visit } from "./history";

describe("history", () => {
  it("goes back and forward, and a new visit drops what was ahead", () => {
    let history = visit(startHistory, { thread: "a", view: null });
    history = visit(history, { thread: "b", view: null });
    history = visit(history, { thread: "b", view: null });
    expect(history.places).toHaveLength(3);

    const back = step(history, -1);
    expect(back?.place.thread).toBe("a");
    expect(step(back!.history, 1)?.place.thread).toBe("b");

    const branched = visit(back!.history, { thread: null, view: "settings" });
    expect(branched.places.map((p) => p.thread ?? p.view)).toEqual([null, "a", "settings"]);
    expect(step(branched, 1)).toBeNull();
  });
});
