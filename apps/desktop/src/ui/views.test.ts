import { describe, expect, it } from "vitest";

import type { AppView } from "./registry";
import { viewButtons, viewShortcuts } from "./views";

const Nothing = () => null;
const view = (id: string, place?: AppView["place"]): AppView => ({ id, title: id, order: 0, place, render: Nothing });

describe("view buttons", () => {
  it("splits the views between the top and the foot of the sidebar, keeping their order", () => {
    const { top, bottom } = viewButtons([view("overview"), view("settings", "bottom"), view("live", "top")]);
    expect(top.map((v) => v.id)).toEqual(["overview", "live"]);
    expect(bottom.map((v) => v.id)).toEqual(["settings"]);
  });

  it("numbers the shortcuts in the order the buttons are drawn, up to nine", () => {
    const views = [view("settings", "bottom"), ...Array.from({ length: 9 }, (_, i) => view(`v${i}`))];
    const shortcuts = viewShortcuts(views);
    expect(shortcuts).toHaveLength(9);
    expect(shortcuts[0]).toMatchObject({ view: { id: "v0" }, shortcut: "Alt+1" });
    // The foot comes after the top, and there is no tenth digit for it.
    expect(shortcuts.map((s) => s.view.id)).not.toContain("settings");
  });
});
