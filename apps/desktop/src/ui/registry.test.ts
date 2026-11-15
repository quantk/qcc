import { describe, expect, it } from "vitest";

import type { Item } from "../api/protocol.gen";
import { UiRegistry, itemKey, matchesShortcut } from "./registry";

const Nothing = () => null;

describe("UiRegistry", () => {
  it("orders tabs and removes a module's contributions as a whole", () => {
    const ui = new UiRegistry();
    ui.activate({
      id: "a",
      activate(ui) {
        ui.threadTab({ id: "late", title: "Late", order: 10, render: Nothing });
        ui.command({ id: "a.run", title: "Run", run: () => {} });
      },
    });
    ui.activate({ id: "b", activate: (ui) => void ui.threadTab({ id: "early", title: "Early", order: 0, render: Nothing }) });
    expect(ui.getSnapshot().tabs.map((tab) => tab.id)).toEqual(["early", "late"]);

    ui.deactivate("a");
    expect(ui.getSnapshot().tabs.map((tab) => tab.id)).toEqual(["early"]);
    expect(ui.getSnapshot().commands).toEqual([]);
    expect(ui.activeModules()).toEqual(["b"]);
  });

  it("replaces what a module registered when it is activated again", () => {
    const ui = new UiRegistry();
    const module = (title: string) => ({
      id: "m",
      activate: (ui: UiRegistry) => void ui.threadTab({ id: "tab", title, order: 0, render: Nothing }),
    });
    ui.activate(module("one"));
    ui.activate(module("two"));
    expect(ui.getSnapshot().tabs.map((tab) => tab.title)).toEqual(["two"]);
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
