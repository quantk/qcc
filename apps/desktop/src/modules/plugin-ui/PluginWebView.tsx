import { useEffect, useRef } from "react";

import type { Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";

type Request = { qcc: 1; id: number; action: string; value?: string };

function isRequest(data: unknown): data is Request {
  if (typeof data !== "object" || data === null) return false;
  const { qcc, id, action, value } = data as Record<string, unknown>;
  return (
    qcc === 1 &&
    typeof id === "number" &&
    typeof action === "string" &&
    (value === undefined || value === null || typeof value === "string")
  );
}

/**
 * A page of a plugin's web bundle. It runs in a sandboxed frame under the plugin's
 * own origin (`qcc-plugin://<plugin-id>`): it cannot reach the app or another
 * plugin's pages, cannot navigate the window or open others, and its responses
 * carry a policy that forbids the network. All it can do is send actions, which
 * go to the plugin that shipped it and to no one else.
 *
 * `allow-same-origin` keeps that origin instead of an anonymous one — WebKit does
 * not load a custom scheme's files into an anonymous document. It is safe here
 * because the origin is never the app's.
 */
export function PluginWebView(props: { plugin: string; view: string; entry: string; title: string; thread?: Thread }) {
  const { plugin, view, entry, title } = props;
  const threadId = props.thread?.id;
  const frame = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const page = frame.current?.contentWindow;
      if (!page || event.source !== page || !isRequest(event.data)) return;
      const { id, action, value } = event.data;
      store()
        .api.call("ui.action", { plugin, view, thread_id: threadId ?? null, action, value: value ?? null })
        .then(
          (result) => page.postMessage({ qcc: 1, id, result: result.tree }, "*"),
          (error: unknown) =>
            page.postMessage({ qcc: 1, id, error: error instanceof Error ? error.message : String(error) }, "*"),
        );
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [plugin, view, threadId]);

  const path = entry.split("/").map(encodeURIComponent).join("/");
  return (
    <iframe
      ref={frame}
      className="plugin-frame"
      title={title}
      sandbox="allow-scripts allow-same-origin"
      referrerPolicy="no-referrer"
      src={`qcc-plugin://${plugin}/${path}`}
    />
  );
}
