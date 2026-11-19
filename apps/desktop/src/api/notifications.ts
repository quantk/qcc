// The system's notifications, shown by the app's backend: the webview has no
// notifications of its own that reach the desktop.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ThreadId } from "./protocol.gen";

/** Show a notification; a click on it, where the system tells, opens `thread`. */
export function showNotification(title: string, body: string, thread: ThreadId | null): Promise<void> {
  return invoke("show_notification", { title, body, thread });
}

/** Calls `handler` with the thread of every notification the user clicks. Returns a function that stops it. */
export function onNotificationClicked(handler: (thread: ThreadId) => void): () => void {
  let stopped = false;
  const unlisten = listen<ThreadId>("notification-clicked", ({ payload }) => {
    if (!stopped) handler(payload);
  });
  return () => {
    stopped = true;
    void unlisten.then((stop) => stop());
  };
}
