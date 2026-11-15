// Projects: named sets of repositories and folders that threads are grouped by.
// The daemon keeps them; this module makes, edits and deletes them, and moves
// threads between them.

import type { Project, ThreadId } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { type UiModule, ui } from "../../ui/registry";
import { MoveThreadDialog } from "./MoveThreadDialog";
import { ProjectDialog } from "./ProjectDialog";

/** Edit `project`, or make a new one starting from `draft`. */
export function openProject(project?: Project, draft?: { name: string; folders: string[] }) {
  ui.showDialog(({ close }) => <ProjectDialog close={close} project={project} draft={draft} />);
}

export function openMoveThread(threadId: ThreadId) {
  ui.showDialog(({ close }) => <MoveThreadDialog threadId={threadId} close={close} />);
}

export const projectsModule: UiModule = {
  id: "core.projects",
  activate(registry) {
    const connected = () => store().getState().daemon.status === "connected";
    registry.command({ id: "project.new", title: "New project", enabled: connected, run: () => openProject() });
    registry.command({
      id: "project.move-thread",
      title: "Move this thread to a project…",
      enabled: () => connected() && store().getState().selected !== null,
      run: () => {
        const selected = store().getState().selected;
        if (selected) openMoveThread(selected);
      },
    });
  },
};
