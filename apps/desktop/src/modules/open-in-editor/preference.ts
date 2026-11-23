// The editor command the user set, shared by the settings screen and the button.

import { useSyncExternalStore } from "react";

import { loadEditor, saveEditor } from "../../state/editor";

function storage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

let current = loadEditor(storage());
const listeners = new Set<() => void>();

export function getEditor(): string {
  return current;
}

export function setEditor(command: string): void {
  const next = command.trim();
  if (next === current) return;
  current = next;
  saveEditor(storage(), next);
  for (const listener of listeners) listener();
}

export function useEditor(): string {
  return useSyncExternalStore((listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }, getEditor);
}
