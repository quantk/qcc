// The live wall: every agent that is working, waiting or just finished, each with
// what it is doing right now, so they can be watched together.

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { LiveWall } from "./LiveWall";

export const LIVE = "live";

export const liveModule: UiModule = {
  id: "core.live",
  activate(ui) {
    ui.appView({ id: LIVE, title: "Live", order: 5, render: LiveWall });
    ui.command({ id: "view.live", title: "Watch the agents at work", shortcut: "Ctrl+Shift+L", run: () => store().openView(LIVE) });
  },
};
