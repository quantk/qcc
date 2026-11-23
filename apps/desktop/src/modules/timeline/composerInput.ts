// How the timeline puts text into the message box under it. Both are this module's,
// so the channel is too: the composer of a thread listens while it is drawn, and
// whatever comes while it is not waits in that thread's draft.

type Listener = (change: (text: string) => string) => void;

const listeners = new Map<string, Listener>();
// Unsent text survives switching between threads.
const drafts = new Map<string, string>();

export function readDraft(threadId: string): string {
  return drafts.get(threadId) ?? "";
}

export function saveDraft(threadId: string, text: string): void {
  drafts.set(threadId, text);
}

/** Called by the composer of a thread while it is drawn. */
export function listenComposer(threadId: string, listener: Listener): () => void {
  listeners.set(threadId, listener);
  return () => {
    if (listeners.get(threadId) === listener) listeners.delete(threadId);
  };
}

/** Change what a thread's composer holds; the composer takes the focus. */
export function changeComposer(threadId: string, change: (text: string) => string): void {
  const listener = listeners.get(threadId);
  if (listener) listener(change);
  else saveDraft(threadId, change(readDraft(threadId)));
}
