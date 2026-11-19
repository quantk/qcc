// What plugins contribute to the window. A plugin describes its views as trees of
// UI nodes; this module turns each contribution into a block like the built-in
// modules' blocks, so a plugin's tab sits next to the timeline (or in the panel
// beside the thread) like any other and the user can move it to any slot that fits it.

import { PuzzleIcon } from "lucide-react";

import type { PluginInfo, Thread, UiContributionsResult } from "../../api/protocol.gen";
import { store } from "../../state/store";
import type { UiModule, UiRegistry } from "../../ui/registry";
import { pluginItemRenderer } from "./PluginItem";
import { PluginView } from "./PluginView";
import { PluginWebView } from "./PluginWebView";

const CONTRIBUTIONS = "plugins.contributions";

/** The module made from what the daemon's plugins contribute right now. */
export function contributionsModule(contributions: UiContributionsResult): UiModule {
  return {
    id: CONTRIBUTIONS,
    activate(ui) {
      // After the built-in contributions, in the order the plugins declared them.
      contributions.views.forEach((view, index) => {
        const id = `${view.plugin}/${view.id}`;
        const order = 100 + index;
        const draw = (thread?: Thread) =>
          view.web ? (
            <PluginWebView plugin={view.plugin} view={view.id} entry={view.web} title={view.title} thread={thread} />
          ) : (
            <PluginView plugin={view.plugin} view={view.id} thread={thread} />
          );
        const hint = `From the plugin ${view.plugin}`;
        if (view.placement === "app") {
          ui.block({
            id,
            title: view.title,
            hint,
            icon: PuzzleIcon,
            scope: "app",
            placement: { slot: "views", order },
            render: () => <div className="plugin-screen">{draw()}</div>,
          });
        } else {
          ui.block({
            id,
            title: view.title,
            hint,
            icon: PuzzleIcon,
            scope: "thread",
            placement: { slot: view.placement === "threadPanel" ? "thread.panel" : "thread.tabs", order },
            when: ({ thread }) => view.scope === "allThreads" || view.providers.includes(thread.provider),
            render: ({ thread }) => draw(thread),
          });
        }
      });
      for (const item of contributions.items) {
        ui.itemRenderer(`extension:${item.kind}`, pluginItemRenderer(item.plugin));
      }
    },
  };
}

export const pluginUiModule: UiModule = {
  id: "core.plugin-ui",
  activate(ui: UiRegistry) {
    // The set of plugins is read again on every connection, and with it their UI.
    let seen: readonly PluginInfo[] | null = null;
    const sync = () => {
      const { plugins, daemon } = store().getState();
      if (plugins === seen || daemon.status !== "connected") return;
      seen = plugins;
      store()
        .api.call("ui.contributions", {})
        .then((contributions) => {
          // Views of hidden plugins are left out; their items are still drawn, since
          // threads made with them stay readable.
          const shown = new Set(plugins.map((plugin) => plugin.id));
          const views = contributions.views.filter((view) => shown.has(view.plugin));
          if (seen === plugins) ui.activate(contributionsModule({ ...contributions, views }));
        })
        .catch(() => {
          seen = null;
        });
    };
    const stop = store().subscribe(sync);
    sync();
    return () => {
      stop();
      ui.deactivate(CONTRIBUTIONS);
    };
  },
};
