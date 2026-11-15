import { describe, expect, it } from "vitest";

import type { Thread, UiContributionsResult } from "../../api/protocol.gen";
import { UiRegistry } from "../../ui/registry";
import { contributionsModule } from "./index";

const contributions: UiContributionsResult = {
  views: [
    { plugin: "notes", id: "all", title: "Notes", placement: "app", scope: "allThreads", providers: [] },
    { plugin: "provider-echo", id: "playground", title: "Echo", placement: "threadTab", scope: "ownThreads", providers: ["echo"] },
    { plugin: "notes", id: "thread", title: "Thread notes", placement: "threadTab", scope: "allThreads", providers: [] },
  ],
  items: [{ plugin: "provider-echo", kind: "provider-echo/card" }],
};

const thread = (provider: string) => ({ provider }) as Thread;

describe("plugin contributions", () => {
  it("become the registrations built-in modules make", () => {
    const ui = new UiRegistry();
    ui.activate(contributionsModule(contributions));
    const { views, tabs, items } = ui.getSnapshot();
    expect(views.map((view) => view.id)).toEqual(["notes/all"]);
    expect(tabs.map((tab) => tab.id)).toEqual(["notes/thread", "provider-echo/playground"]);
    expect([...items.keys()]).toEqual(["extension:provider-echo/card"]);

    const shown = (provider: string) =>
      tabs
        .filter((tab) => tab.when?.({ thread: thread(provider), timeline: undefined, provider: undefined }) ?? true)
        .map((tab) => tab.id);
    expect(shown("echo")).toEqual(["notes/thread", "provider-echo/playground"]);
    expect(shown("codex")).toEqual(["notes/thread"]);
  });

  it("are replaced as a whole when the plugins change", () => {
    const ui = new UiRegistry();
    ui.activate(contributionsModule(contributions));
    ui.activate(contributionsModule({ views: [], items: [] }));
    const { views, tabs, items } = ui.getSnapshot();
    expect([views.length, tabs.length, items.size]).toEqual([0, 0, 0]);
  });
});
