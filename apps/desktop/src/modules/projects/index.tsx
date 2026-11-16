// Projects: named sets of repositories and folders that threads are grouped by.
// The daemon keeps them; this module makes, edits and deletes them, and moves
// threads between them.

import { FolderPlusIcon, Settings2Icon } from "lucide-react";

import type { Project, ThreadId } from "../../api/protocol.gen";
import { baseName } from "../../util/format";
import { store } from "../../state/store";
import { type UiModule, ui } from "../../ui/registry";
import { MoveThreadDialog } from "./MoveThreadDialog";
import { ProjectDialog } from "./ProjectDialog";

/** Edit `project`, or make a new one starting from `draft`. */
function openProject(project?: Project, draft?: { name: string; folders: string[] }) {
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
    registry.groupAction({
      id: "edit-project",
      order: 20,
      when: (group) => group.kind === "project",
      label: (group) => (group.kind === "project" ? `Edit ${group.project.name}` : "Edit project"),
      title: "Name and folders",
      icon: Settings2Icon,
      run: (group) => {
        if (group.kind === "project") openProject(group.project);
      },
    });
    registry.groupAction({
      id: "folder-to-project",
      order: 30,
      when: (group) => group.kind === "folder",
      label: (group) => (group.kind === "folder" ? `Make ${baseName(group.folder)} a project` : "Make a project"),
      title: "Make this folder a project",
      icon: FolderPlusIcon,
      onHover: true,
      run: (group) => {
        if (group.kind === "folder") openProject(undefined, { name: baseName(group.folder), folders: [group.folder] });
      },
    });
    registry.threadMenuItem({
      id: "move-to-project",
      title: "Move to project…",
      order: 20,
      group: "edit",
      when: () => store().getState().projects.length > 0,
      run: (thread) => openMoveThread(thread.id),
    });
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
