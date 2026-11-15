// What plugins contribute to the window. A plugin describes its views as trees of
// UI nodes; this module turns each contribution into the same registrations the
// built-in modules make, so a plugin's tab sits next to the timeline like any other.

import type { PluginInfo, UiContributionsResult } from "../../api/protocol.gen";
import { store } from "../../state/store";
import type { UiModule, UiRegistry } from "../../ui/registry";
import { pluginItemRenderer } from "./PluginItem";
import { PluginView } from "./PluginView";

const CONTRIBUTIONS = "plugins.contributions";

/** The module made from what the daemon's plugins contribute right now. */
export function contributionsModule(contributions: UiContributionsResult): UiModule {
  return {
    id: CONTRIBUTIONS,
    activate(ui) {
      for (const view of contributions.views) {
        const id = `${view.plugin}/${view.id}`;
        if (view.placement === "app") {
          ui.appView({
            id,
            title: view.title,
            order: 100,
            render: () => (
              <div className="plugin-screen">
                <PluginView plugin={view.plugin} view={view.id} />
              </div>
            ),
          });
        } else {
          ui.threadTab({
            id,
            title: view.title,
            order: 100,
            when: ({ thread }) => view.scope === "allThreads" || view.providers.includes(thread.provider),
            render: ({ thread }) => <PluginView plugin={view.plugin} view={view.id} thread={thread} />,
          });
        }
      }
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
          if (seen === plugins) ui.activate(contributionsModule(contributions));
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
