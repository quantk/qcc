import type { ProviderInfo, Thread } from "../../api/protocol.gen";
import { threadMcpRows } from "../../state/mcp";
import { store, useApp } from "../../state/store";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { McpStateMark } from "./McpState";
import { MCP_VIEW } from "./open";

/** The MCP servers of the thread's session, and switches that keep qcc's off in this thread. */
export function ThreadMcp({ thread, provider }: { thread: Thread; provider: ProviderInfo | undefined }) {
  const settings = useApp((state) => state.settings?.settings);
  const rows = threadMcpRows(thread, settings);
  const live = provider?.capabilities.mcp_toggle ?? false;

  return (
    <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 py-5">
      <p className="text-[13px] text-muted-foreground">
        What the agent says about the MCP servers of its session. Servers set up in qcc can be turned off for this
        thread alone, not the threads it starts;{" "}
        {live
          ? "the switch takes effect at once."
          : "the agent's session starts again with the change, after the running turn if there is one, and goes on where it was."}{" "}
        The agent's own servers are shown as it reports them: change those in the agent itself.
      </p>
      <ul className="flex flex-col divide-y rounded-lg border">
        {rows.map((row) => (
          <li key={row.name} className="flex flex-col gap-1 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <span className={`font-mono text-sm font-semibold${row.off ? " text-muted-foreground" : ""}`}>{row.name}</span>
              {row.ours ? (
                <Badge variant="secondary" title="Set up in qcc">
                  qcc
                </Badge>
              ) : (
                <Badge variant="outline" title="From the agent's own configuration">
                  agent
                </Badge>
              )}
              <span className="flex-1" />
              {row.status ? (
                <McpStateMark state={row.status.state} />
              ) : row.off ? (
                <McpStateMark state="disabled" />
              ) : (
                <span className="text-xs text-muted-foreground">Not reported</span>
              )}
              {/* An agent's own server a thread kept off earlier can be let go, not turned off again. */}
              {(row.ours || row.off) && (
                <Switch
                  aria-label={`Use ${row.name} in this thread`}
                  title={row.off ? "Off in this thread" : "On in this thread"}
                  checked={!row.off}
                  onCheckedChange={(on) => void store().setThreadMcp(thread.id, row.name, on)}
                />
              )}
            </div>
            {row.pending && (
              <p className="text-xs text-muted-foreground">Kept off in this thread; the session has not taken the change yet.</p>
            )}
            {row.pendingOn && !live && (
              <p className="text-xs text-muted-foreground">On again from the thread's next session.</p>
            )}
            {row.status?.error && !row.off && (
              <p className="line-clamp-3 text-xs break-all text-destructive" title={row.status.error}>
                {row.status.error}
              </p>
            )}
            {row.status?.state === "needs-auth" && !row.off && (
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
