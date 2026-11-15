// Applies the interface scale to the window and lets views follow it.

import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useSyncExternalStore } from "react";

import { clampZoom, loadZoom, saveZoom } from "../../state/zoom";

function storage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

let current = loadZoom(storage());
const listeners = new Set<() => void>();

/**
 * Scales the whole webview the way a browser's zoom does, plugin frames included.
 * Outside Tauri (the frontend alone in a browser) the root element is scaled instead.
 */
function apply(zoom: number): void {
  if (isTauri()) {
    getCurrentWebview()
      .setZoom(zoom)
      .catch((error: unknown) => console.warn("cannot set the zoom", error));
  } else {
    document.documentElement.style.setProperty("zoom", String(zoom));
  }
}

export function getZoom(): number {
  return current;
}

export function setZoom(zoom: number): void {
  const next = clampZoom(zoom);
  if (next === current) return;
  current = next;
  saveZoom(storage(), next);
  apply(next);
  for (const listener of listeners) listener();
}

/** Puts the saved scale on the window; called once when the app starts. */
export function restoreZoom(): void {
  if (current !== 1) apply(current);
}

export function useZoom(): number {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getZoom,
  );
}
