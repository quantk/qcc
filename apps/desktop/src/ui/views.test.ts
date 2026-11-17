import { describe, expect, it } from "vitest";

import type { Block } from "./registry";
import { viewShortcuts } from "./views";

const Nothing = () => null;
const view = (id: string): Block => ({ id, title: id, scope: "app", placement: { slot: "views", order: 0 }, render: Nothing });

describe("view shortcuts", () => {
  it("number the buttons in the order they are drawn, the top row first, up to nine", () => {
    const top = Array.from({ length: 9 }, (_, i) => view(`v${i}`));
    const shortcuts = viewShortcuts(top, [view("settings")]);
    expect(shortcuts).toHaveLength(9);
    expect(shortcuts[0]).toMatchObject({ view: { id: "v0" }, shortcut: "Alt+1" });
    // The foot comes after the top, and there is no tenth digit for it.
    expect(shortcuts.map((s) => s.view.id)).not.toContain("settings");
  });

  it("leave out blocks that need a thread", () => {
    const thread: Block = { id: "t", title: "t", scope: "thread", placement: { slot: "views", order: 0 }, render: Nothing };
    expect(viewShortcuts([thread, view("a")], []).map((s) => s.shortcut)).toEqual(["Alt+1"]);
  });
});
