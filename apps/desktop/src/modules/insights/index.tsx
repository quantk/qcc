// The side panel's built-in tabs: how full the context is and what was spent,
// what the agent did, and the thread's facts in one place.

import type { UiModule } from "../../ui/registry";
import { ActivityPanel } from "./ActivityPanel";
import { DetailsPanel } from "./DetailsPanel";
import { UsagePanel } from "./UsagePanel";

export const insightsModule: UiModule = {
  id: "insights",
  activate(ui) {
    ui.threadPanel({ id: "insights.usage", title: "Usage", order: 0, render: UsagePanel });
    ui.threadPanel({ id: "insights.activity", title: "Activity", order: 10, render: ActivityPanel });
    ui.threadPanel({ id: "insights.details", title: "Details", order: 20, render: DetailsPanel });
  },
};
