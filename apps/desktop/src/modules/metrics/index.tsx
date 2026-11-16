// What agents spent and how their turns went, over time and across threads.

import type { UiModule } from "../../ui/registry";
import { MetricsView } from "./MetricsView";

export const metricsModule: UiModule = {
  id: "metrics",
  activate(ui) {
    ui.appView({ id: "metrics", title: "Metrics", order: 20, render: MetricsView });
  },
};
