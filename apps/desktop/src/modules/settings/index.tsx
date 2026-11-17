// The settings screen: the daemon's settings (agents' programs, defaults and
// environment), the plugins, and preferences of this window such as its scale.

import { SettingsIcon } from "lucide-react";

import { store } from "../../state/store";
import { zoomIn, zoomOut } from "../../state/zoom";
import type { UiModule } from "../../ui/registry";
import { SettingsView } from "./SettingsView";
import { getZoom, restoreZoom, setZoom } from "./zoom";

export const SETTINGS = "settings";

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
  },
};
