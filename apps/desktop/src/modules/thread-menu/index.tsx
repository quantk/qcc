// What a thread's right-click menu offers: opening, naming, read state, copying
// its id or directory, stopping and archiving. Other modules add their own
// entries the same way (moving it to a project lives with projects).

import { isUnread } from "../../state/reads";
import { folderOf } from "../../state/selectors";
import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { ArchiveDialog } from "./ArchiveDialog";
import { IsolateDialog } from "./IsolateDialog";
import { RenameDialog } from "./RenameDialog";

function copy(text: string, what: string) {
  navigator.clipboard
    .writeText(text)
    .then(() => store().notify(`Copied the ${what}`))
    .catch(() => store().notify(`Could not copy the ${what}: ${text}`));
}

export const threadMenuModule: UiModule = {
  id: "core.thread-menu",
  activate(ui) {
    ui.threadMenuItem({ id: "open", title: "Open", order: 0, group: "open", run: (t) => store().select(t.id) });
    ui.threadMenuItem({
      id: "rename",
      title: "Rename…",
      order: 10,
      group: "edit",
      run: (thread) => ui.showDialog(({ close }) => <RenameDialog thread={thread} close={close} />),
    });
    ui.threadMenuItem({
      id: "read",
      title: (thread) => (isUnread(thread, store().getState().seen) ? "Mark as read" : "Mark as unread"),
      order: 30,
      group: "read",
      when: (thread) => !thread.archived && !thread.parent && Boolean(thread.last_turn_at),
      run: (thread) => {
        if (isUnread(thread, store().getState().seen)) store().markRead(thread.id);
        else store().markUnread(thread.id);
      },
    });
    ui.threadMenuItem({ id: "copy-id", title: "Copy thread id", order: 40, group: "copy", run: (t) => copy(t.id, "thread id") });
    ui.threadMenuItem({
      id: "copy-dir",
      title: "Copy directory path",
      order: 41,
      group: "copy",
      run: (thread) => copy(thread.cwd, "directory path"),
    });
    ui.threadMenuItem({
      id: "copy-source",
      title: "Copy source folder path",
      order: 42,
      group: "copy",
      when: (thread) => folderOf(thread) !== thread.cwd,
      run: (thread) => copy(folderOf(thread), "source folder path"),
    });
    ui.threadMenuItem({
      id: "isolate",
      title: "Move into a worktree…",
      order: 25,
      group: "edit",
      when: (thread) => !thread.archived && !thread.environment && thread.status !== "active",
      run: (thread) => ui.showDialog(({ close }) => <IsolateDialog thread={thread} close={close} />),
    });
    ui.threadMenuItem({
      id: "stop",
      title: "Stop the running turn",
      order: 50,
      group: "run",
      when: (thread) => thread.status === "active",
      run: (thread) => void store().stop(thread.id),
    });
    ui.threadMenuItem({
      id: "archive",
      title: "Archive",
      order: 60,
      group: "run",
      destructive: true,
      when: (thread) => !thread.archived && thread.status !== "active",
      run: (thread) =>
        void store()
          .archive(thread.id, false)
          .then((result) => {
            if (result?.refused) {
              const refused = result.refused;
              ui.showDialog(({ close }) => <ArchiveDialog thread={thread} refused={refused} close={close} />);
            }
          }),
    });
  },
};
