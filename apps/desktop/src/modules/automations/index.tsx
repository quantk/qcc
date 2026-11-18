// Automations: prompts qcc sends an agent on a schedule. A screen lists them with
// their next and latest runs and the threads those started; a thread started by one
// says so in its header and leads back to it.

import { CalendarClockIcon } from "lucide-react";

import type { Thread } from "../../api/protocol.gen";
import { automationName } from "../../state/automations";
import { store, useApp } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { Button } from "@/components/ui/button";
import { AutomationsView } from "./AutomationsView";
import { AUTOMATIONS_VIEW, openAutomation, showAutomation } from "./open";

/** In the header of a thread an automation started: which one, and a way to it. */
function AutomationLink({ thread }: { thread: Thread }) {
  const id = thread.automation ?? "";
  const name = useApp((state) => automationName(state.automations, id));
  return (
    <Button
      variant="outline"
      size="sm"
      title={name ? `Started by the automation ${name}` : "Started by an automation that was deleted since"}
      disabled={!name}
      onClick={() => showAutomation(id)}
    >
      <CalendarClockIcon /> {name ?? "Deleted automation"}
    </Button>
  );
}

export const automationsModule: UiModule = {
  id: "core.automations",
  activate(registry) {
    const connected = () => store().getState().daemon.status === "connected";
    registry.block({
      id: AUTOMATIONS_VIEW,
      title: "Automations",
      hint: "Prompts sent to agents on a schedule",
      scope: "app",
      placement: { slot: "views.foot", order: 85 },
      icon: CalendarClockIcon,
      render: AutomationsView,
    });
    registry.threadAction({
      id: "automation",
      order: -9,
      when: ({ thread }) => Boolean(thread.automation),
      render: ({ thread }) => <AutomationLink thread={thread} />,
    });
    registry.command({ id: "automations.open", title: "Automations", run: () => store().openView(AUTOMATIONS_VIEW) });
    registry.command({ id: "automations.new", title: "New automation", enabled: connected, run: () => openAutomation() });
  },
};
