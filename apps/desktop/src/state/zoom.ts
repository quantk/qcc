// The interface scale: the steps a browser offers, kept in this window's local storage.

/** The scales Ctrl+Plus and Ctrl+Minus step through. */
export const ZOOM_LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;

export const MIN_ZOOM = ZOOM_LEVELS[0];
export const MAX_ZOOM = ZOOM_LEVELS[ZOOM_LEVELS.length - 1]!;

const STORAGE_KEY = "qcc.zoom";

export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));
}

/** The next step up. A scale between steps goes to the nearest step above it. */
export function zoomIn(zoom: number): number {
  return ZOOM_LEVELS.find((level) => level > zoom + 1e-6) ?? MAX_ZOOM;
}

/** The next step down. A scale between steps goes to the nearest step below it. */
export function zoomOut(zoom: number): number {
  return [...ZOOM_LEVELS].reverse().find((level) => level < zoom - 1e-6) ?? MIN_ZOOM;
}

export function formatZoom(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}

/** Reads what was saved; storage can be missing or refuse, which means 100%. */
export function loadZoom(storage: Pick<Storage, "getItem"> | undefined): number {
  try {
    const saved = storage?.getItem(STORAGE_KEY);
    return saved ? clampZoom(Number(saved)) : 1;
  } catch {
    return 1;
  }
}

export function saveZoom(storage: Pick<Storage, "setItem"> | undefined, zoom: number): void {
  try {
    storage?.setItem(STORAGE_KEY, String(zoom));
  } catch {
    // A scale that is not remembered is still applied to this session.
  }
}
