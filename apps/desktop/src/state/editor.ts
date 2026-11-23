// Opening a thread's directory in an editor: which directory, whether it can be
// opened, and the editor command the user set, kept in this window's local storage.

import type { Thread } from "../api/protocol.gen";

const STORAGE_KEY = "qcc.editor";

/** The directory to open, or why there is none to open. */
export type EditorTarget = { dir: string } | { reason: string };

/**
 * A thread's directory is its `cwd`: once its environment is ready, that is the
 * environment's directory (the worktree), and before, the folder it is made from,
 * which is not where the thread works, so it is not offered.
 */
export function editorTarget(thread: Thread, exists: boolean | undefined): EditorTarget {
  switch (thread.environment?.state) {
    case "pending":
      return { reason: "The thread's directory is not made yet" };
    case "failed":
      return { reason: "The thread's directory could not be made" };
    case "removed":
      return { reason: "The thread's directory was removed" };
  }
  if (exists === false) return { reason: `${thread.cwd} does not exist any more` };
  return { dir: thread.cwd };
}

/** The command saved; empty means "find an editor". */
export function loadEditor(storage: Pick<Storage, "getItem"> | undefined): string {
  try {
    return storage?.getItem(STORAGE_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
}

export function saveEditor(storage: Pick<Storage, "setItem" | "removeItem"> | undefined, command: string): void {
  try {
    const trimmed = command.trim();
    if (trimmed) storage?.setItem(STORAGE_KEY, trimmed);
    else storage?.removeItem(STORAGE_KEY);
  } catch {
    // A command that is not remembered is still used in this session.
  }
}
