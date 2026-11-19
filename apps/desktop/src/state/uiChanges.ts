// Plugins tell the app when one of their views shows something else (`ui.changed`).
// The app counts the changes of each view, and of each thread's copy of a thread
// view; a view drawn with a different count asks the plugin again.

import type { UiChangedNotification } from "../api/protocol.gen";

export type UiChanges = Readonly<Record<string, number>>;

/** The key a view's changes are counted under: all its copies, or one thread's. */
export function uiChangeKey(plugin: string, view: string, threadId?: string | null): string {
  return threadId ? `${plugin}/${view}@${threadId}` : `${plugin}/${view}`;
}

/** The counts after a change. */
export function noteUiChange(changes: UiChanges, change: Pick<UiChangedNotification, "plugin" | "view" | "thread_id">): UiChanges {
  const key = uiChangeKey(change.plugin, change.view, change.thread_id);
  return { ...changes, [key]: (changes[key] ?? 0) + 1 };
}

/** How often a view drawn for `threadId` (or for no thread) has changed. */
export function uiChangeCount(changes: UiChanges, plugin: string, view: string, threadId?: string | null): number {
  const all = changes[uiChangeKey(plugin, view)] ?? 0;
  return threadId ? all + (changes[uiChangeKey(plugin, view, threadId)] ?? 0) : all;
}
