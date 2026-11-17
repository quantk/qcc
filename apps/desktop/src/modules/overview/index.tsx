// The control panel: all agents on one screen.

import { LayoutDashboardIcon } from "lucide-react";

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { Overview } from "./Overview";

export const OVERVIEW = "overview";

export const overviewModule: UiModule = {
  id: "core.overview",
  activate(ui) {
    ui.block({ id: OVERVIEW, title: "Overview", scope: "app", placement: { slot: "views", order: 0 }, icon: LayoutDashboardIcon, render: Overview });
    ui.command({ id: "view.overview", title: "Go to the overview", shortcut: "Ctrl+0", run: () => store().openView(OVERVIEW) });
  },
};
