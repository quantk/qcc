// Notifications plugins ask the app to show (`notification.show`): whether one is
// worth showing given what the user is looking at.

import type { NotificationShowNotification, ThreadId } from "../api/protocol.gen";

/** What the user sees of the app when a notification comes. */
export type Looking = {
  /** The app's window has focus and is not hidden. */
  focused: boolean;
  /** The thread open in the window, if one is. */
  selected: ThreadId | null;
};

/**
 * Whether to show a notification. One about the thread the user is looking at in a
 * focused window tells nothing new, if the plugin says so (`quiet_when_shown`).
 */
export function worthShowing(notification: NotificationShowNotification, looking: Looking): boolean {
  const thread = notification.thread_id ?? null;
  const onScreen = looking.focused && thread !== null && looking.selected === thread;
  return !(notification.quiet_when_shown === true && onScreen);
}
