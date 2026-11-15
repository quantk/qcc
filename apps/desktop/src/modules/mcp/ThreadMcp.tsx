import type { Thread } from "../../api/protocol.gen";
import { store, useApp } from "../../state/store";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { McpStateMark } from "./McpState";
import { MCP_VIEW } from "./open";

/** The MCP servers of the thread's session, as its agent reports them. */
export function ThreadMcp({ thread }: { thread: Thread }) {
  // Selected as is: a fresh `{}` on every call would render forever.
  const configured = useApp((state) => state.settings?.settings.mcp_servers);
  const ours = configured ?? {};
  const servers = thread.mcp_servers ?? [];
  return (
    <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 py-5">
      <p className="text-[13px] text-muted-foreground">
        What the agent says about the MCP servers of its current session, its own configuration's included.
      </p>
      <ul className="flex flex-col divide-y rounded-lg border">
        {servers.map((server) => (
          <li key={server.name} className="flex flex-col gap-1 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <span className="font-mono text-sm font-semibold">{server.name}</span>
              {server.name in ours ? (
                <Badge variant="secondary" title="Set up in qcc">
                  qcc
                </Badge>
              ) : (
                <Badge variant="outline" title="From the agent's own configuration">
                  agent
                </Badge>
              )}
              <span className="flex-1" />
              <McpStateMark state={server.state} />
            </div>
            {server.error && (
              <p className="line-clamp-3 text-xs break-all text-destructive" title={server.error}>
                {server.error}
              </p>
            )}
            {server.state === "needs-auth" && (
              <p className="text-xs text-muted-foreground">Sign in to it in the agent itself; the next session picks it up.</p>
            )}
          </li>
        ))}
      </ul>
      <Button variant="ghost" size="sm" className="self-start" onClick={() => store().openView(MCP_VIEW)}>
        Manage MCP servers
      </Button>
    </div>
  );
}
