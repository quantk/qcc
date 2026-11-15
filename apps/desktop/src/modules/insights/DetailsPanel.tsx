import { CopyIcon } from "lucide-react";

import { store, useApp } from "../../state/store";
import type { ThreadContext } from "../../ui/registry";
import { relativeTime, tailPath } from "../../util/format";
import { useNow } from "../../util/useNow";
import { Fact, Section } from "./Facts";
import { Button } from "@/components/ui/button";

const date = (ts: number) =>
  new Date(ts).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

export function DetailsPanel({ thread, provider, parent, children }: ThreadContext) {
  const now = useNow(60_000);
  const mcp = thread.mcp_servers ?? [];
  const project = useProjectName(thread.project ?? null);
  return (
    <>
      <Section title="Agent">
        <Fact label="Provider">{provider?.name ?? thread.provider}</Fact>
        <Fact label="Model">{thread.model ?? "agent default"}</Fact>
        <Fact label="Effort">{thread.effort ?? "agent default"}</Fact>
        <Fact label="Permissions">{thread.permission_mode ?? "agent config"}</Fact>
        <Fact label="Status">{thread.archived ? "archived" : thread.status}</Fact>
        {thread.provider_session && (
          <Fact label="Session" title={thread.provider_session}>
            <span className="inline-flex items-center gap-1">
              <code className="truncate font-mono text-xs">{thread.provider_session.slice(0, 13)}</code>
              <Button
                variant="ghost"
                size="icon-xs"
                title="Copy the session id"
                onClick={() => void copy(thread.provider_session!)}
              >
                <CopyIcon />
              </Button>
            </span>
          </Fact>
        )}
      </Section>
      <Section title="Where">
        <Fact label="Directory" title={thread.cwd}>
          {tailPath(thread.cwd, 32)}
        </Fact>
        {thread.environment && (
          <>
            <Fact label="Environment">
              {thread.environment.id} · {thread.environment.state}
            </Fact>
            <Fact label="Source" title={thread.environment.source_dir}>
              {tailPath(thread.environment.source_dir, 32)}
            </Fact>
          </>
        )}
        {project && <Fact label="Project">{project}</Fact>}
      </Section>
      <Section title="Thread">
        <Fact label="Created" title={date(thread.created_at)}>
          {ago(thread.created_at, now)}
        </Fact>
        <Fact label="Updated" title={date(thread.updated_at)}>
          {ago(thread.updated_at, now)}
        </Fact>
        {(thread.conversations ?? 0) > 0 && <Fact label="Fresh starts">{thread.conversations}</Fact>}
        {parent && (
          <Fact label="Started by" title={parent.title ?? parent.id}>
            {parent.title ?? "New thread"}
          </Fact>
        )}
        {children.length > 0 && <Fact label="Threads started">{children.length}</Fact>}
        {mcp.length > 0 && <Fact label="MCP servers">{mcp.length}</Fact>}
        <Fact label="Events">{thread.last_seq}</Fact>
      </Section>
    </>
  );
}

function ago(ts: number, now: number): string {
  const when = relativeTime(ts, now);
  return /^\d+[mhd]$/.test(when) ? `${when} ago` : when;
}

function useProjectName(id: string | null): string | null {
  return useApp((state) => (id ? (state.projects.find((project) => project.id === id)?.name ?? null) : null));
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    store().notify("Session id copied");
  } catch {
    // The clipboard is not ours here; the id is in the tooltip.
  }
}
