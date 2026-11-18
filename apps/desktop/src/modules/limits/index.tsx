// Usage limits of the agents' subscriptions: how much of the 5-hour and weekly
// windows each provider's account has spent and when they start over. Limits are
// the account's, not a thread's, so they stand in the sidebar, always in view; the
// details open from there, and a block with all of them can be put anywhere.

import { GaugeIcon } from "lucide-react";

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { LimitsGlance, LimitsView } from "./LimitsViews";

const LIMITS_VIEW = "limits.view";

export const limitsModule: UiModule = {
  id: "core.limits",
  activate(registry) {
    registry.block({
      id: "limits.glance",
      title: "Usage limits",
      hint: "How much of each agent's subscription limits is spent",
      scope: "app",
      placement: { slot: "sidebar", order: 10 },
      icon: GaugeIcon,
      render: LimitsGlance,
    });
    registry.block({
      id: LIMITS_VIEW,
      title: "Usage limits in full",
      hint: "Every window of each agent's subscription, with when it resets",
      scope: "app",
      placement: { slot: null, order: 88 },
      icon: GaugeIcon,
      render: LimitsView,
    });
    registry.command({ id: "limits.open", title: "Usage limits", run: () => store().openView(LIMITS_VIEW) });
    registry.command({
      id: "limits.refresh",
      title: "Refresh usage limits",
      enabled: () => store().getState().daemon.status === "connected",
      run: () => void store().refreshLimits(),
    });
  },
};
