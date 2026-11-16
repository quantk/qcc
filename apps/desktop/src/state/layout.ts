// How wide the two sides of the window are. The user drags their edges; the widths
// are remembered, and kept within bounds so a stale or hand-edited value cannot hide
// the thread or squeeze a side to nothing.

export type Bounds = { min: number; max: number; initial: number };

/** The thread list at the left. */
export const SIDEBAR: Bounds = { min: 200, max: 520, initial: 280 };

/** The side panel at the right of a thread. Diffs want room, so it may grow wide. */
export const SIDE_PANEL_WIDTH: Bounds = { min: 260, max: 1000, initial: 300 };

/** What the thread keeps at least when a side panel beside it grows. */
export const THREAD_MIN = 420;

/** A whole number of pixels within `bounds`; anything that is not a number is the initial width. */
export function clampWidth(value: unknown, bounds: Bounds): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return bounds.initial;
  return Math.round(Math.min(bounds.max, Math.max(bounds.min, value)));
}

/**
 * The width a drag asks for. `edge` says which side of the element the handle is on:
 * the right edge grows with the pointer moving right, the left edge with it moving left.
 */
export function draggedWidth(start: number, startX: number, x: number, edge: "left" | "right", bounds: Bounds): number {
  const delta = edge === "right" ? x - startX : startX - x;
  return clampWidth(start + delta, bounds);
}
