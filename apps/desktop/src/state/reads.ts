// What the user has seen of each thread: the end of the latest turn they had in
// front of them. A thread whose turn ended later has news for them.

import type { Thread } from "../api/protocol.gen";

/** Thread id → `last_turn_at` of the turn last seen. */
export type Seen = Readonly<Record<string, number>>;

/**
 * Whether a thread finished a turn the user has not looked at. Threads started by
 * other agents are left out: their answers go to the agent that started them.
 */
export function isUnread(thread: Thread, seen: Seen): boolean {
  if (thread.archived || thread.parent || thread.status === "active" || !thread.last_turn_at) return false;
  return thread.last_turn_at > (seen[thread.id] ?? 0);
}

/** Record that the user sees `thread` as it is now. Unchanged when nothing is new. */
export function markSeen(seen: Seen, thread: Thread): Seen {
  const at = thread.last_turn_at ?? 0;
  return (seen[thread.id] ?? 0) >= at ? seen : { ...seen, [thread.id]: at };
}

/** Make a thread look new again, as if its latest turn had not been seen. */
export function markUnseen(seen: Seen, thread: Thread): Seen {
  const at = thread.last_turn_at ?? 0;
  return at === 0 ? seen : { ...seen, [thread.id]: at - 1 };
}

/**
 * The first time the app runs there is no record: what is there already counts
 * as seen, rather than every old thread showing up as news.
 */
export function seenEverything(threads: readonly Thread[]): Seen {
  return Object.fromEntries(threads.map((thread) => [thread.id, thread.last_turn_at ?? 0]));
}
