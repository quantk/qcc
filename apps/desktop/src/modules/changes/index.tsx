// What the agent changed in its working directory: in full as a tab, and beside the
// timeline in the side panel. A thread in an environment shows what it changed
// there; a thread in a directory of its own, what is uncommitted in it.

import { changesShown } from "../../state/selectors";
import { store } from "../../state/store";
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
      when: ({ thread }) => changesShown(thread, store().getState().allPlugins),
      render: ({ thread }) => <ChangesView thread={thread} />,
    });
    ui.threadPanel({
      id: "changes",
      title: "Changes",
      order: 5,
      when: ({ thread }) => changesShown(thread, store().getState().allPlugins),
      render: ChangesPanel,
    });
  },
};
