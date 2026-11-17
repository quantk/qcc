// Back and forward between the threads and screens visited, as in a browser:
// Alt+Left and Alt+Right, the mouse's back and forward buttons, the palette. And
// the panel at the right of a thread, which belongs to the same frame. And Alt+1 to
// Alt+9 for the views, in the order their buttons are drawn.

import { type Layout, arrange } from "../../state/blocks";
import { store } from "../../state/store";
import type { Block, UiModule } from "../../ui/registry";
import { openViewCommand, viewShortcuts } from "../../ui/views";

/** `MouseEvent.button` of the side buttons most mice have. */
const MOUSE_BACK = 3;
const MOUSE_FORWARD = 4;

export const navigationModule: UiModule = {
  id: "core.navigation",
  activate(ui) {
    ui.command({ id: "nav.back", title: "Back", shortcut: "Alt+Left", run: () => store().navigate(-1) });
    ui.command({ id: "nav.forward", title: "Forward", shortcut: "Alt+Right", run: () => store().navigate(1) });
    ui.command({
      id: "view.side-panel",
      title: "Show or hide the side panel",
      shortcut: "Ctrl+B",
      run: () => store().toggleSidePanel(),
    });

    // The views come and go with modules, plugins and the user's layout, and their
    // shortcuts with them.
    let seen: { blocks: readonly Block[]; layout: Layout } | null = null;
    let removeShortcuts: (() => void)[] = [];
    const syncShortcuts = () => {
      const blocks = ui.getSnapshot().blocks;
      const layout = store().getState().blockLayout;
      // Registering a command notifies this listener again; the blocks are the same then.
      if (seen?.blocks === blocks && seen.layout === layout) return;
      seen = { blocks, layout };
      const { slots } = arrange(blocks, layout);
      removeShortcuts.forEach((remove) => remove());
      removeShortcuts = viewShortcuts(slots.views, slots["views.foot"]).map(({ view, shortcut }) =>
        ui.command({ id: openViewCommand(view.id), title: `Open ${view.title}`, shortcut, run: () => store().openView(view.id) }),
      );
    };
    const stopUi = ui.subscribe(syncShortcuts);
    const stopStore = store().subscribe(syncShortcuts);
    syncShortcuts();

    // The webview would otherwise take the side buttons for its own history and
    // reload the page; they are ours on every phase of the click.
    const swallow = (event: MouseEvent) => {
      if (event.button === MOUSE_BACK || event.button === MOUSE_FORWARD) event.preventDefault();
    };
    const onUp = (event: MouseEvent) => {
      swallow(event);
      if (event.button === MOUSE_BACK) store().navigate(-1);
      if (event.button === MOUSE_FORWARD) store().navigate(1);
    };
    window.addEventListener("mousedown", swallow);
    window.addEventListener("auxclick", swallow);
    window.addEventListener("mouseup", onUp);
    return () => {
      stopUi();
      stopStore();
      removeShortcuts.forEach((remove) => remove());
      window.removeEventListener("mousedown", swallow);
      window.removeEventListener("auxclick", swallow);
      window.removeEventListener("mouseup", onUp);
    };
  },
};
