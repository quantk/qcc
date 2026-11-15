import { useEffect, useState } from "react";

import { store } from "../../state/store";
import { Disclosure, NodeView, StatusMark } from "../../ui/kit";
import type { ItemRendererProps } from "../../ui/registry";

// A settled item never changes, so what its plugin drew for it is kept.
const drawn = new Map<string, unknown>();

/** Makes the renderer of one plugin's extension items. */
export function pluginItemRenderer(plugin: string) {
  return function PluginItem({ item }: ItemRendererProps) {
    const cacheKey = `${item.id}:${item.status}`;
    const [tree, setTree] = useState<unknown>(() => drawn.get(cacheKey) ?? null);
    const [failed, setFailed] = useState(false);
    const kind = item.type === "extension" ? item.kind : "";
    const payload = item.type === "extension" ? item.payload : null;

    useEffect(() => {
      if (!kind || drawn.has(cacheKey)) return;
      let stale = false;
      store()
        .api.call("ui.render_item", { plugin, kind, payload: payload ?? null })
        .then((result) => {
          drawn.set(cacheKey, result.tree);
          if (!stale) setTree(result.tree);
        })
        .catch(() => {
          if (!stale) setFailed(true);
        });
      return () => {
        stale = true;
      };
      // The payload belongs to the item: the cache key covers it.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [cacheKey, kind]);

    if (tree !== null) {
      return (
        <div className="plugin-item">
          <NodeView tree={tree} />
        </div>
      );
    }
    // Until the plugin answers, or if it cannot, the item is shown as its data.
    return (
      <Disclosure
        className="tool"
        head={
          <>
            <span className="label">{kind}</span>
            {failed ? <span className="tag failed">not drawn</span> : <StatusMark status={item.status} />}
          </>
        }
      >
        <pre className="output">{JSON.stringify(payload, null, 2)}</pre>
      </Disclosure>
    );
  };
}
