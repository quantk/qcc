// The settings screen: the daemon's settings (agents' programs, defaults and
// environment), the plugins, and preferences of this window such as its scale.

import { SettingsIcon } from "lucide-react";

import { defaultLayout } from "../../state/blocks";
import { store } from "../../state/store";
import { zoomIn, zoomOut } from "../../state/zoom";
import type { UiModule } from "../../ui/registry";
import { LAYOUT_SECTION } from "./LayoutSettings";
import { SettingsView } from "./SettingsView";
import { getZoom, restoreZoom, setZoom } from "./zoom";

export const SETTINGS = "settings";

/** Scroll the settings to the layout once the screen is drawn; it may take a few frames. */
function showLayoutSection(tries = 10) {
  const section = document.getElementById(LAYOUT_SECTION);
  if (section) section.scrollIntoView({ block: "start" });
  else if (tries > 0) requestAnimationFrame(() => showLayoutSection(tries - 1));
}

export const settingsModule: UiModule = {
  id: "core.settings",
  activate(ui) {
    restoreZoom();
    ui.block({
      id: SETTINGS,
      title: "Settings",
      scope: "app",
      placement: { slot: "views.foot", order: 100 },
      icon: SettingsIcon,
      render: SettingsView,
    });
    ui.command({ id: "view.settings", title: "Open settings", shortcut: "Ctrl+,", run: () => store().openView(SETTINGS) });
    // The shortcuts a browser uses. Ctrl+0 already opens the overview, so resetting
    // the scale is left to the palette and the settings screen.
    ui.command({ id: "zoom.in", title: "Zoom in", shortcut: "Ctrl+Plus", run: () => setZoom(zoomIn(getZoom())) });
    ui.command({ id: "zoom.out", title: "Zoom out", shortcut: "Ctrl+Minus", run: () => setZoom(zoomOut(getZoom())) });
    ui.command({ id: "zoom.reset", title: "Reset zoom to 100%", run: () => setZoom(1) });
    // The layout is reached from the menu of any block, so a block hidden or moved
    // by mistake is always one step from coming back, settings included.
    ui.command({
      id: "layout.edit",
      title: "Arrange blocks",
      run: () => {
        store().openView(SETTINGS);
        showLayoutSection();
      },
    });
    ui.command({ id: "layout.reset", title: "Reset the layout of blocks", run: () => store().setBlockLayout(defaultLayout) });
  },
};
