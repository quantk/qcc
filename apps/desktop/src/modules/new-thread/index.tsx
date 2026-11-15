import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { NewThreadDialog } from "./NewThreadDialog";

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
