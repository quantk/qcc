// What the agent changed in its own working directory, for threads that have one:
// in full as a tab, and beside the timeline in the side panel.

import type { UiModule } from "../../ui/registry";
import { ChangesPanel } from "./ChangesPanel";
import { ChangesView } from "./ChangesView";

export const changesModule: UiModule = {
  id: "core.changes",
  activate(ui) {
    ui.threadTab({
      id: "changes",
      title: "Changes",
      order: 10,
      when: ({ thread }) => thread.environment?.state === "ready",
      render: ({ thread }) => <ChangesView thread={thread} />,
    });
    ui.threadPanel({
      id: "changes",
      title: "Changes",
      order: 5,
      when: ({ thread }) => thread.environment?.state === "ready",
      render: ChangesPanel,
    });
  },
};
