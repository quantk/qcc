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
    ui.block({
      id: "changes",
      title: "Changes",
      hint: "The diff of the thread's work in full",
      scope: "thread",
      placement: { slot: "thread.tabs", order: 10 },
      when: ({ thread }) => changesShown(thread, store().getState().allPlugins),
      render: ({ thread }) => <ChangesView thread={thread} />,
    });
    // The same changes, compact: a list of files to keep in view beside the timeline.
    ui.block({
      id: "changes.panel",
      title: "Changes",
      hint: "The files the thread changed, compact",
      scope: "thread",
      placement: { slot: "thread.panel", order: 5 },
      when: ({ thread }) => changesShown(thread, store().getState().allPlugins),
      render: ChangesPanel,
    });
  },
};
