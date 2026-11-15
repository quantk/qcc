import { useCallback, useEffect, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { NodeView } from "../../ui/kit";

const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** One view of a plugin: asks the plugin to draw it and sends the user's actions back. */
export function PluginView({ plugin, view, thread }: { plugin: string; view: string; thread?: Thread }) {
  const [tree, setTree] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const threadId = thread?.id;
  // The plugin is asked again when the thread settles: its state may have moved.
  const settled = thread ? `${thread.status}:${thread.last_turn_status ?? ""}` : "";

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
        if (!stale) setError(message(error));
      });
    return () => {
      stale = true;
    };
  }, [plugin, view, threadId, settled]);

  const act = useCallback(
    (action: string, value?: string) => {
      setBusy(true);
      store()
        .api.call("ui.action", { plugin, view, thread_id: threadId ?? null, action, value: value ?? null })
        .then((result) => {
          setTree(result.tree);
          setError(null);
        })
        .catch((error: unknown) => setError(message(error)))
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
