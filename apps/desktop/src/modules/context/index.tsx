// What a thread's context window holds: a side panel tab with the window's parts,
// as the agent says or, failing that, as reckoned from its usage.

import { GaugeIcon } from "lucide-react";

import type { UiModule } from "../../ui/registry";
import { ContextPanel } from "./ContextPanel";

export const contextModule: UiModule = {
  id: "context",
  activate(ui) {
    ui.block({
      id: "context.window",
      title: "Context",
      hint: "How full the context window is and what it holds",
      scope: "thread",
      placement: { slot: "thread.panel", order: 5 },
      icon: GaugeIcon,
      render: ContextPanel,
    });
  },
};
