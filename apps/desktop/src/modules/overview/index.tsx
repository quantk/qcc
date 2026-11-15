// The control panel: all agents on one screen.

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { Overview } from "./Overview";

export const OVERVIEW = "overview";

export const overviewModule: UiModule = {
  id: "core.overview",
  activate(ui) {
    ui.appView({ id: OVERVIEW, title: "Overview", order: 0, render: Overview });
    ui.command({ id: "view.overview", title: "Go to the overview", shortcut: "Ctrl+0", run: () => store().openView(OVERVIEW) });
  },
};
