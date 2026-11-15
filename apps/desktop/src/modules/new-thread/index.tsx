import type { ProjectId } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { type UiModule, ui } from "../../ui/registry";
import { NewThreadDialog } from "./NewThreadDialog";

/** Open the dialog, starting in a project. */
export function openNewThread(preset: { project?: ProjectId } = {}) {
  ui.showDialog(({ close }) => <NewThreadDialog close={close} project={preset.project} />);
}

export const NEW_THREAD = "thread.new";

export const newThreadModule: UiModule = {
  id: "core.new-thread",
  activate(ui) {
    ui.command({
      id: NEW_THREAD,
      title: "New thread",
      shortcut: "Ctrl+N",
      enabled: () => store().getState().daemon.status === "connected",
      run: () => ui.showDialog(NewThreadDialog),
    });
  },
};
