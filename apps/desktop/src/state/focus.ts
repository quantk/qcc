// Bringing a place of a thread's changes into view: a file, and a line of it as it
// is now. A plugin's view asks for it (a finding's `file:line`); the Changes tab
// scrolls there once the file's diff is read.

import type { DiffFile } from "./diff";

export type ChangesFocus = {
  threadId: string;
  path: string;
  /** In the file as it is now; null: the file itself. */
  line: number | null;
  /** Tells a new request for the same place from the last one. */
  seq: number;
};

/**
 * The key of the row to scroll to (`f:<path>` for the file's header,
 * `l:<path>:<index>` for a line), or null while the file is not read. A line the
 * diff does not show falls back to the header.
 */
export function focusRow(focus: ChangesFocus, file: DiffFile | undefined): string | null {
  if (focus.line === null) return `f:${focus.path}`;
  if (!file) return null;
  const index = file.lines.findIndex((line) => line.newNo === focus.line && line.kind !== "del");
  return index < 0 ? `f:${focus.path}` : `l:${focus.path}:${index}`;
}
