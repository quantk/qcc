// What the agent changed in its own working directory, for threads that have one.

import type { UiModule } from "../../ui/registry";
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
  },
};
