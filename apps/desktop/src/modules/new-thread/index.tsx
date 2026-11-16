import { PlusIcon } from "lucide-react";

import type { ProjectId } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { type UiModule, ui } from "../../ui/registry";
import { NewThreadDialog } from "./NewThreadDialog";

/** Open the dialog, starting in a project. */
function openNewThread(preset: { project?: ProjectId } = {}) {
  ui.showDialog(({ close }) => <NewThreadDialog close={close} project={preset.project} />);
}

export const newThreadModule: UiModule = {
  id: "core.new-thread",
  activate(registry) {
    registry.command({
      id: "thread.new",
      title: "New thread",
      shortcut: "Ctrl+N",
      enabled: () => store().getState().daemon.status === "connected",
      run: () => openNewThread(),
    });
    registry.groupAction({
      id: "new-thread-in-project",
      order: 10,
      when: (group) => group.kind === "project",
      label: (group) => (group.kind === "project" ? `New thread in ${group.project.name}` : "New thread"),
      title: "New thread in this project",
      icon: PlusIcon,
      run: (group) => openNewThread({ project: group.kind === "project" ? group.project.id : undefined }),
    });
  },
};
