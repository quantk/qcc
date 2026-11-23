// Opening directories in the user's editor, done by the app's backend: the webview
// cannot start programs.

import { invoke } from "@tauri-apps/api/core";

/** Open `path` with `command`, or a found editor when it is empty. Resolves to what opened it. */
export function openInEditor(path: string, command: string): Promise<string> {
  return invoke("open_in_editor", { path, command: command.trim() || null });
}

/** The editor used when none is set, as its full path. */
export function detectEditor(): Promise<string | null> {
  return invoke("detect_editor");
}

export function directoryExists(path: string): Promise<boolean> {
  return invoke("directory_exists", { path });
}
