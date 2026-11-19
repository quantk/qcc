import { useCallback, useEffect, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { store, useApp } from "../../state/store";
import { uiChangeCount } from "../../state/uiChanges";
import { NodeView } from "../../ui/kit";
import { errorMessage } from "../../util/format";


/** One view of a plugin: asks the plugin to draw it and sends the user's actions back. */
export function PluginView({ plugin, view, thread }: { plugin: string; view: string; thread?: Thread }) {
  const [tree, setTree] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const threadId = thread?.id;
  // The plugin is asked again when the thread settles: its state may have moved.
  const settled = thread ? `${thread.status}:${thread.last_turn_status ?? ""}` : "";
  // And when the plugin says the view shows something else now.
  const changed = useApp((state) => uiChangeCount(state.uiChanges, plugin, view, threadId));

  useEffect(() => {
    let stale = false;
    store()
      .api.call("ui.render", { plugin, view, thread_id: threadId ?? null })
      .then((result) => {
        if (stale) return;
        setTree(result.tree);
        setError(null);
      })
      .catch((error: unknown) => {
        if (!stale) setError(errorMessage(error));
      });
    return () => {
      stale = true;
    };
  }, [plugin, view, threadId, settled, changed]);

  const act = useCallback(
    (action: string, value?: string) => {
      setBusy(true);
      store()
        .api.call("ui.action", { plugin, view, thread_id: threadId ?? null, action, value: value ?? null })
        .then((result) => {
          setTree(result.tree);
          setError(null);
        })
        .catch((error: unknown) => setError(errorMessage(error)))
        .finally(() => setBusy(false));
    },
    [plugin, view, threadId],
  );

  return (
    <div className="plugin-view">
      {error && <div className="notice error">{error}</div>}
      {tree === null && !error ? <p className="hint">Loading…</p> : <NodeView tree={tree} onAction={act} busy={busy} />}
    </div>
  );
}
