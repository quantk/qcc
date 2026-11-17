import { describe, expect, it } from "vitest";

import type { Item, Thread } from "../api/protocol.gen";
import { UiRegistry, blockApplies, itemKey, matchesShortcut } from "./registry";

const Nothing = () => null;

describe("UiRegistry", () => {
  it("offers blocks and removes a module's contributions as a whole", () => {
    const ui = new UiRegistry();
    ui.activate({
      id: "a",
      activate(ui) {
        ui.block({ id: "late", title: "Late", scope: "thread", placement: { slot: "thread.tabs", order: 10 }, render: Nothing });
        ui.command({ id: "a.run", title: "Run", run: () => {} });
      },
    });
    ui.activate({
      id: "b",
      activate: (ui) => void ui.block({ id: "early", title: "Early", scope: "app", placement: { slot: "views", order: 0 }, render: Nothing }),
    });
    expect(ui.getSnapshot().blocks.map((block) => block.id)).toEqual(["late", "early"]);

    ui.deactivate("a");
    expect(ui.getSnapshot().blocks.map((block) => block.id)).toEqual(["early"]);
    expect(ui.getSnapshot().commands).toEqual([]);
    expect(ui.activeModules()).toEqual(["b"]);
  });

  it("replaces what a module registered when it is activated again", () => {
    const ui = new UiRegistry();
    const module = (title: string) => ({
      id: "m",
      activate: (ui: UiRegistry) =>
        void ui.block({ id: "tab", title, scope: "thread", placement: { slot: "thread.tabs", order: 0 }, render: Nothing }),
    });
    ui.activate(module("one"));
    ui.activate(module("two"));
    expect(ui.getSnapshot().blocks.map((block) => block.title)).toEqual(["two"]);
  });

  it("asks a thread's block whether it applies, and shows a block that needs nothing anywhere", () => {
    const context = { thread: { provider: "echo" } as Thread, timeline: undefined, provider: undefined, parent: undefined, children: [] };
    const placement = { slot: "thread.tabs", order: 0 } as const;
    const never = { id: "n", title: "N", scope: "thread", placement, when: () => false, render: Nothing } as const;
    const always = { id: "a", title: "A", scope: "app", placement, render: Nothing } as const;
    expect(blockApplies(never, context)).toBe(false);
    expect(blockApplies(always, context)).toBe(true);
  });

  it("lets a later renderer take over an item kind and gives it back", () => {
    const ui = new UiRegistry();
    const Core = () => null;
    const Custom = () => null;
    ui.itemRenderer("command", Core);
    const remove = ui.itemRenderer("command", Custom);
    expect(ui.getSnapshot().items.get("command")).toBe(Custom);
    remove();
    expect(ui.getSnapshot().items.get("command")).toBe(Core);
  });

  it("keys plugin items by their kind", () => {
    const item: Item = { id: "i", status: "completed", type: "extension", kind: "notes/card", payload: null };
    expect(itemKey(item)).toBe("extension:notes/card");
    expect(itemKey({ id: "i", status: "completed", type: "fileRead", path: "a" })).toBe("fileRead");
  });

  it("matches shortcuts exactly", () => {
    const event = (init: Partial<KeyboardEvent>) => ({ ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...init }) as KeyboardEvent;
    expect(matchesShortcut(event({ key: "n", ctrlKey: true }), "Ctrl+N")).toBe(true);
    expect(matchesShortcut(event({ key: "N", metaKey: true }), "Ctrl+N")).toBe(true);
    expect(matchesShortcut(event({ key: "n", ctrlKey: true, shiftKey: true }), "Ctrl+N")).toBe(false);
    expect(matchesShortcut(event({ key: "n" }), "Ctrl+N")).toBe(false);
    expect(matchesShortcut(event({ key: "=", ctrlKey: true }), "Ctrl+Plus")).toBe(true);
    expect(matchesShortcut(event({ key: "+", ctrlKey: true, shiftKey: true }), "Ctrl+Plus")).toBe(true);
    expect(matchesShortcut(event({ key: "-", ctrlKey: true }), "Ctrl+Minus")).toBe(true);
    expect(matchesShortcut(event({ key: "-" }), "Ctrl+Minus")).toBe(false);
  });
});

describe("thread menu", () => {
  it("keeps entries in order and lets a module take its own out", () => {
    const registry = new UiRegistry();
    registry.activate({
      id: "a",
      activate(ui) {
        ui.threadMenuItem({ id: "archive", title: "Archive", order: 60, run: () => {} });
        ui.threadMenuItem({ id: "open", title: "Open", order: 0, run: () => {} });
      },
    });
    registry.activate({
      id: "b",
      activate(ui) {
        ui.threadMenuItem({ id: "move", title: "Move", order: 20, run: () => {} });
      },
    });
    expect(registry.getSnapshot().menu.map((item) => item.id)).toEqual(["open", "move", "archive"]);
    registry.deactivate("b");
    expect(registry.getSnapshot().menu.map((item) => item.id)).toEqual(["open", "archive"]);
  });
});

describe("group actions", () => {
  it("keeps the buttons of sidebar groups in order and takes them out with their module", () => {
    const registry = new UiRegistry();
    const action = (id: string, order: number) => ({ id, order, label: () => id, title: id, icon: Nothing, run: () => {} });
    registry.activate({ id: "a", activate: (ui) => void ui.groupAction(action("edit", 20)) });
    registry.activate({ id: "b", activate: (ui) => void ui.groupAction(action("new", 10)) });
    expect(registry.getSnapshot().groupActions.map((item) => item.id)).toEqual(["new", "edit"]);
    registry.deactivate("b");
    expect(registry.getSnapshot().groupActions.map((item) => item.id)).toEqual(["edit"]);
  });
});
