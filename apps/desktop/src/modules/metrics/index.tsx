// What agents spent, how their turns went and what they used, over time and across threads.

import { ChartColumnIcon } from "lucide-react";

import type { UiModule } from "../../ui/registry";
import { MetricsView } from "./MetricsView";

export const metricsModule: UiModule = {
  id: "metrics",
  activate(ui) {
    ui.block({ id: "metrics", title: "Metrics", scope: "app", placement: { slot: "views", order: 20 }, icon: ChartColumnIcon, render: MetricsView });
  },
};
