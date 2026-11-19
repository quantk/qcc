// Notifications plugins ask for (the bundled `notify` plugin: an agent finished,
// failed or waits for an answer), shown as the system's own. What to tell is the
// plugin's call; the app only knows what the user is looking at, so it leaves out
// what is already on screen. A click on one opens its thread.

import { onNotificationClicked, showNotification } from "../../api/notifications";
import { worthShowing } from "../../state/notifications";
import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";

function windowFocused(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

export const notificationsModule: UiModule = {
  id: "core.notifications",
  activate() {
    const app = store();
    const stopShowing = app.api.onNotification((notification) => {
      if (notification.method !== "notification.show") return;
      const params = notification.params;
      const looking = { focused: windowFocused(), selected: app.getState().selected };
      if (!worthShowing(params, looking)) return;
      showNotification(params.title, params.body, params.thread_id ?? null).catch((error: unknown) => {
        console.warn("cannot show a notification", error);
      });
    });
    const stopClicks = onNotificationClicked((thread) => app.select(thread));
    return () => {
      stopShowing();
      stopClicks();
    };
  },
};
