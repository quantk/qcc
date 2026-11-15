// Back and forward between the threads and screens visited, as in a browser:
// Alt+Left and Alt+Right, the mouse's back and forward buttons, the palette.

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";

/** `MouseEvent.button` of the side buttons most mice have. */
const MOUSE_BACK = 3;
const MOUSE_FORWARD = 4;

export const navigationModule: UiModule = {
  id: "core.navigation",
  activate(ui) {
    ui.command({ id: "nav.back", title: "Back", shortcut: "Alt+Left", run: () => store().navigate(-1) });
    ui.command({ id: "nav.forward", title: "Forward", shortcut: "Alt+Right", run: () => store().navigate(1) });

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
      window.removeEventListener("mousedown", swallow);
      window.removeEventListener("auxclick", swallow);
      window.removeEventListener("mouseup", onUp);
    };
  },
};
