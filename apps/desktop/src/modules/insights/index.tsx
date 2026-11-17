// The side panel's built-in tabs: how full the context is and what was spent,
// what the agent did, and the thread's facts in one place.

import type { ThreadBlock, UiModule } from "../../ui/registry";
import { ActivityPanel } from "./ActivityPanel";
import { DetailsPanel } from "./DetailsPanel";
import { UsagePanel } from "./UsagePanel";

export const insightsModule: UiModule = {
  id: "insights",
  activate(ui) {
    const panel = (id: string, title: string, order: number, render: ThreadBlock["render"]) =>
      ui.block({ id, title, scope: "thread", placement: { slot: "thread.panel", order }, render });
    panel("insights.usage", "Usage", 0, UsagePanel);
    panel("insights.activity", "Activity", 10, ActivityPanel);
    panel("insights.details", "Details", 20, DetailsPanel);
  },
};
