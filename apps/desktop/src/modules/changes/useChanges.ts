import { useEffect, useMemo, useSyncExternalStore } from "react";

import type { Thread } from "../../api/protocol.gen";
import { ChangesReaders, ThreadChangesReader, totals } from "../../state/changes";
import { settledEdits } from "../../state/stats";
import { store, useApp } from "../../state/store";
import { errorMessage } from "../../util/format";

/** Edits in a burst land within moments of each other; one read covers them. */
const SETTLE_MS = 800;

/** A view that opens within this of the last read shows what that read found. */
const FRESH_MS = 2000;

const readers = new ChangesReaders(
  (threadId) =>
    new ThreadChangesReader(
      threadId,
      {
        changes: (id) => store().api.call("thread.changes", { thread_id: id }).then(({ changes }) => changes),
        diff: (params) => store().api.call("thread.diff", params),
      },
      errorMessage,
    ),
);

/** The reader of a thread's changes, shared with every other view of it while held. */
function useReader(threadId: string): ThreadChangesReader {
  const reader = readers.get(threadId);
  useEffect(() => readers.hold(threadId)[1], [threadId]);
  return reader;
}

/**
 * What the agent changed in its working directory: the list of files, read again
 * when the agent settles an edit or a command, when a turn ends, and on `reload`;
 * the diff of each file is read when a view shows it (`reader.want`).
 */
export function useChanges(thread: Thread) {
  const reader = useReader(thread.id);
  const state = useSyncExternalStore(reader.subscribe, reader.getState);
  const active = thread.status === "active";
  const entries = useApp((app) => app.timelines[thread.id]?.entries);
  const edits = useMemo(() => settledEdits(entries ?? []), [entries]);

  // A view opening reads the list afresh, unless another view just did.
  useEffect(() => reader.refresh(FRESH_MS), [reader]);
  // Later reads wait for a burst of edits to pass; views that saw the same edits read once.
  useEffect(() => reader.follow(`${active}:${edits}`, SETTLE_MS), [reader, active, edits]);

  const sum = useMemo(() => totals(state.changes), [state.changes]);
  return {
    reader,
    state,
    changes: state.changes,
    error: state.error,
    loading: state.loading,
    active,
    totals: sum,
    reload: () => reader.reload(),
  };
}
