import { describe, expect, it } from "vitest";

import type { Thread, UiContributionsResult } from "../../api/protocol.gen";
import { blocksIn } from "../../state/blocks";
import { UiRegistry, blockApplies } from "../../ui/registry";
import { contributionsModule } from "./index";

const contributions: UiContributionsResult = {
  views: [
    { plugin: "notes", id: "all", title: "Notes", placement: "app", scope: "allThreads", providers: [] },
    { plugin: "provider-echo", id: "playground", title: "Echo", placement: "threadTab", scope: "ownThreads", providers: ["echo"] },
    { plugin: "notes", id: "thread", title: "Thread notes", placement: "threadTab", scope: "allThreads", providers: [] },
    { plugin: "notes", id: "board", title: "Board", placement: "app", scope: "allThreads", web: "index.html", providers: [] },
    { plugin: "reviewers", id: "reviewers", title: "Reviewers", placement: "threadPanel", scope: "allThreads", providers: [] },
  ],
  items: [{ plugin: "provider-echo", kind: "provider-echo/card" }],
};

const thread = (provider: string) => ({ provider }) as Thread;

describe("plugin contributions", () => {
  it("become blocks in the screens and the thread tabs", () => {
    const ui = new UiRegistry();
    ui.activate(contributionsModule(contributions));
    const { blocks, items } = ui.getSnapshot();
    const views = blocksIn(blocks, {}, "views");
    const tabs = blocksIn(blocks, {}, "thread.tabs");
    expect(views.map((view) => view.id)).toEqual(["notes/all", "notes/board"]);
    expect(tabs.map((tab) => tab.id)).toEqual(["provider-echo/playground", "notes/thread"]);
    expect(blocksIn(blocks, {}, "thread.panel").map((panel) => panel.id)).toEqual(["reviewers/reviewers"]);
    expect([...items.keys()]).toEqual(["extension:provider-echo/card"]);

    const shown = (provider: string) =>
      tabs
        .filter((tab) => blockApplies(tab, { thread: thread(provider), timeline: undefined, provider: undefined, parent: undefined, children: [] }))
        .map((tab) => tab.id);
    expect(shown("echo")).toEqual(["provider-echo/playground", "notes/thread"]);
    expect(shown("codex")).toEqual(["notes/thread"]);
  });

  it("are replaced as a whole when the plugins change", () => {
    const ui = new UiRegistry();
    ui.activate(contributionsModule(contributions));
    ui.activate(contributionsModule({ views: [], items: [] }));
    const { blocks, items } = ui.getSnapshot();
    expect([blocks.length, items.size]).toEqual([0, 0]);
  });
});
