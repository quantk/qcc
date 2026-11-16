import { useEffect, useMemo, useState } from "react";

import type { EnvironmentChanges, Thread } from "../../api/protocol.gen";
import { type DiffFile, parseDiff } from "../../state/diff";
import { settledEdits } from "../../state/stats";
import { store, useApp } from "../../state/store";
import { errorMessage } from "../../util/format";

export type Loaded = { changes: EnvironmentChanges; files: DiffFile[]; truncated: boolean };

/** Edits in a burst land within moments of each other; one read covers them. */
const SETTLE_MS = 800;

/**
 * What the agent changed in its working directory, read from the daemon. It is read
 * again when the agent settles an edit or a command, when a turn ends, and on
 * `reload`, so a view beside a working agent keeps up with it.
 */
export function useChanges(thread: Thread) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const active = thread.status === "active";
  const entries = useApp((state) => state.timelines[thread.id]?.entries);
  const edits = useMemo(() => settledEdits(entries ?? []), [entries]);

  useEffect(() => {
    let stale = false;
    // The first read is at once; later ones wait for a burst of edits to pass.
    const timer = setTimeout(
      () => {
        setLoading(true);
        const { api } = store();
        const params = { thread_id: thread.id };
        Promise.all([api.call("thread.changes", params), api.call("thread.diff", params)])
          .then(([{ changes }, { diff, truncated }]) => {
            if (stale) return;
            setLoaded({ changes, files: parseDiff(diff), truncated: truncated ?? false });
            setError(null);
          })
          .catch((error: unknown) => {
            if (!stale) setError(errorMessage(error));
          })
          .finally(() => {
            if (!stale) setLoading(false);
          });
      },
      loaded ? SETTLE_MS : 0,
    );
    return () => {
      stale = true;
      clearTimeout(timer);
    };
    // `loaded` only picks the delay; reading again because it changed would loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thread.id, active, edits, refresh]);

  const totals = useMemo(() => {
    const files = loaded?.changes.files ?? [];
    return {
      additions: files.reduce((sum, file) => sum + (file.additions ?? 0), 0),
      deletions: files.reduce((sum, file) => sum + (file.deletions ?? 0), 0),
    };
  }, [loaded]);

  return { loaded, error, loading, active, totals, reload: () => setRefresh((count) => count + 1) };
}
