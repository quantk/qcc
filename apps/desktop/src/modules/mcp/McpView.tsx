import { PencilIcon, PlusIcon } from "lucide-react";
import { useMemo } from "react";

import type { McpServerSettings, McpState } from "../../api/protocol.gen";
import {
  type McpUsage,
  agentOwnMcpServers,
  mcpServersOf,
  mcpUsage,
  transportSummary,
  withMcpServer,
} from "../../state/mcp";
import { providerName } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { plural } from "../../util/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { openMcpServer } from "./open";
import { McpStateMark } from "./McpState";

const ORDER: McpState[] = ["failed", "needs-auth", "starting", "ready", "disabled"];

/** The servers set up in qcc, and those agents bring from their own configuration. */
export function McpView() {
  const settings = useApp((state) => state.settings?.settings);
  const threadMap = useApp((state) => state.threads);
  const threads = useMemo(() => Object.values(threadMap), [threadMap]);
  const ours = mcpServersOf(settings);
  const theirs = agentOwnMcpServers(threads, settings);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">MCP servers</h1>
        <span className="flex-1" />
        <Button size="sm" onClick={() => openMcpServer()}>
          <PlusIcon /> Add server
        </Button>
      </header>
      <div className="flex w-full max-w-3xl flex-col gap-6 overflow-auto px-6 pt-6 pb-10">
        <section className="flex flex-col gap-2">
          <div>
            <h2 className="text-base font-semibold">Set up in qcc</h2>
            <p className="text-[13px] text-muted-foreground">
              Every agent gets these on top of the servers of its own configuration. A change reaches the sessions that
              start after it; a running thread gets it when its session starts again.
            </p>
          </div>
          {ours.length === 0 && (
            <p className="rounded-lg border border-dashed p-4 text-[13px] text-muted-foreground">
              None yet. Agents use only the servers of their own configuration.
            </p>
          )}
          {ours.map(([name, server]) => (
            <ServerCard key={name} name={name} server={server} usage={mcpUsage(threads, name)} />
          ))}
        </section>
        {theirs.length > 0 && (
          <section className="flex flex-col gap-2">
            <div>
              <h2 className="text-base font-semibold">From the agents' own configuration</h2>
              <p className="text-[13px] text-muted-foreground">
                Servers running threads report that qcc did not give them. Change them in the agent itself (
                <code>claude mcp</code>, <code>~/.codex/config.toml</code>, opencode's config).
              </p>
            </div>
            {theirs.map((server) => (
              <article key={server.name} className="flex flex-col gap-1.5 rounded-lg border p-4">
                <header className="flex flex-wrap items-center gap-2">
                  <h3 className="font-mono text-sm font-semibold">{server.name}</h3>
                  {server.providers.map((provider) => (
                    <ProviderBadge key={provider} provider={provider} />
                  ))}
                </header>
                <UsageLine usage={server.usage} />
              </article>
            ))}
          </section>
        )}
      </div>
    </div>
  );
}

function ProviderBadge({ provider }: { provider: string }) {
  const name = useApp((state) => providerName(state.allPlugins, provider));
  return <Badge variant="outline">{name}</Badge>;
}

function ServerCard({ name, server, usage }: { name: string; server: McpServerSettings; usage: McpUsage }) {
  const enabled = !server.disabled;
  const toggle = (on: boolean) =>
    void store().changeSettings((settings) => withMcpServer(settings, name, { ...server, disabled: on ? undefined : true }));
  return (
    <article className={`flex flex-col gap-1.5 rounded-lg border p-4${enabled ? "" : " opacity-70"}`}>
      <header className="flex items-center gap-2">
        <h3 className="font-mono text-sm font-semibold">{name}</h3>
        <Badge variant="secondary">{server.transport.type === "stdio" ? "program" : "HTTP"}</Badge>
        {!enabled && <Badge variant="outline">off</Badge>}
        <span className="flex-1" />
        <Button variant="ghost" size="icon-sm" aria-label={`Edit ${name}`} title="Edit" onClick={() => openMcpServer(name, server)}>
          <PencilIcon />
        </Button>
        <Switch
          aria-label={`Give agents ${name}`}
          title={enabled ? "New sessions get it" : "Kept, but no session gets it"}
          checked={enabled}
          onCheckedChange={toggle}
        />
      </header>
      <code className="truncate text-xs text-muted-foreground" title={transportSummary(server.transport)}>
        {transportSummary(server.transport)}
      </code>
      {enabled ? (
        <UsageLine usage={usage} />
      ) : (
        <p className="text-xs text-muted-foreground">Off: sessions started from now on do not get it.</p>
      )}
    </article>
  );
}

/** How a server is doing across the threads that report it. */
function UsageLine({ usage }: { usage: McpUsage }) {
  const states = ORDER.filter((state) => usage.counts[state]);
  if (states.length === 0) {
    return <p className="text-xs text-muted-foreground">No running thread has reported it yet.</p>;
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap gap-x-4 gap-y-1">
        {states.map((state) => (
          <McpStateMark
            key={state}
            state={state}
            suffix={` in ${plural(usage.counts[state] ?? 0, "thread")}`}
          />
        ))}
      </div>
      {usage.errors.map((error) => (
        <p key={error} className="line-clamp-3 text-xs break-all text-destructive" title={error}>
          {error}
        </p>
      ))}
    </div>
  );
}
