// Where the user has been, for going back and forward as in a browser.

import type { ThreadId } from "../api/protocol.gen";

/** A thread, or one of the app's screens when no thread is open. */
export type Place = { thread: ThreadId | null; view: string | null };

export type History = { readonly places: readonly Place[]; readonly index: number };

const LIMIT = 100;

export const startHistory: History = { places: [{ thread: null, view: null }], index: 0 };

export function samePlace(a: Place | undefined, b: Place): boolean {
  return a !== undefined && a.thread === b.thread && (a.thread !== null || a.view === b.view);
}

/** Going somewhere new drops the places ahead, as a browser does. */
export function visit(history: History, place: Place): History {
  if (samePlace(history.places[history.index], place)) return history;
  const places = [...history.places.slice(0, history.index + 1), place].slice(-LIMIT);
  return { places, index: places.length - 1 };
}

/** One step back (`-1`) or forward (`1`); null when there is nowhere to go. */
export function step(history: History, by: -1 | 1): { history: History; place: Place } | null {
  const index = history.index + by;
  const place = history.places[index];
  return place ? { history: { ...history, index }, place } : null;
}
