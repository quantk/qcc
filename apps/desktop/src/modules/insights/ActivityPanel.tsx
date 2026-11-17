import { useMemo } from "react";

import { activityOf, turnSummary } from "../../state/stats";
import { type ToolRow, byServer, ofKind, rowLabel, toolsOfTimeline } from "../../state/tools";
import type { ThreadContext } from "../../ui/registry";
import { baseName, elapsed, plural, relativePath } from "../../util/format";
import { useNow } from "../../util/useNow";
import { Fact, Quiet, Section } from "./Facts";

/** Tools and files listed by name before the rest is only counted. */
const LISTED = 6;

/** Echo and quick answers take less than a second; "0s" would read as nothing. */
const duration = (ms: number) => (ms > 0 && ms < 1000 ? "<1s" : elapsed(ms));

export function ActivityPanel({ thread, timeline }: ThreadContext) {
  const activity = useMemo(() => activityOf(timeline?.entries ?? []), [timeline?.entries]);
  const tools = useMemo(() => toolsOfTimeline(timeline?.entries ?? []), [timeline?.entries]);
  const turns = useMemo(() => turnSummary(timeline?.stats.turns ?? []), [timeline?.stats.turns]);
  const now = useNow(turns.running !== null ? 1000 : 60_000);
  if (!timeline) return <Quiet>Loading…</Quiet>;
  const ended = turns.completed + turns.failed + turns.interrupted;

  return (
    <>
      <Section title="Turns" aside={turns.count > 0 ? String(turns.count) : undefined}>
        {turns.count === 0 ? (
          <Quiet>No turns yet.</Quiet>
        ) : (
          <>
            {turns.running !== null && <Fact label="Running for">{elapsed(now - turns.running)}</Fact>}
            <Fact label="Completed">{turns.completed}</Fact>
            {turns.failed > 0 && <Fact label="Failed">{turns.failed}</Fact>}
            {turns.interrupted > 0 && <Fact label="Interrupted">{turns.interrupted}</Fact>}
            <Fact label="Time working">{duration(turns.working)}</Fact>
            {ended > 0 && <Fact label="Average turn">{duration(turns.working / ended)}</Fact>}
            {ended > 1 && <Fact label="Longest turn">{duration(turns.longest)}</Fact>}
          </>
        )}
      </Section>
      <Section title="Conversation">
        <Fact label="Your messages">{activity.userMessages}</Fact>
        <Fact label="Agent messages">{activity.agentMessages}</Fact>
        {activity.questions > 0 && (
          <Fact label="Questions and approvals">
            {activity.questions}
            {activity.unanswered > 0 && ` (${activity.unanswered} unanswered)`}
          </Fact>
        )}
        {activity.errors > 0 && <Fact label="Errors">{activity.errors}</Fact>}
      </Section>
      <Section title="Work">
        <Fact label="Commands">
          {activity.commands}
          {activity.failedCommands > 0 && ` (${activity.failedCommands} failed)`}
        </Fact>
        <Fact label="Files changed">{activity.filesChanged.length}</Fact>
        <Fact label="Files read">{activity.filesRead.length}</Fact>
        <Fact label="Searches">{activity.searches}</Fact>
      </Section>
      {activity.filesChanged.length > 0 && (
        <Section title="Changed files">
          <ul className="flex flex-col gap-0.5 text-sm">
            {activity.filesChanged.slice(0, LISTED).map((path) => (
              <li key={path} className="truncate" title={path}>
                {relativePath(path, thread.cwd) === path ? baseName(path) : relativePath(path, thread.cwd)}
              </li>
            ))}
          </ul>
          {activity.filesChanged.length > LISTED && (
            <Quiet>and {plural(activity.filesChanged.length - LISTED, "more file")}</Quiet>
          )}
        </Section>
      )}
      <ToolSection title="Tools" rows={tools.filter((row) => row.kind === "tool")} noun="tool" />
      <McpSection rows={tools} />
      <ToolSection title="Skills used" rows={ofKind(tools, "skills")} noun="skill" />
      <ToolSection title="Programs run" rows={ofKind(tools, "commands")} noun="program" />
    </>
  );
}

/** How a use went, for its title: failures and time. */
function useTitle(row: ToolRow): string {
  const parts = [`${row.calls} ${row.calls === 1 ? "use" : "uses"}`];
  if (row.failed > 0) parts.push(`${row.failed} failed`);
  if (row.declined > 0) parts.push(`${row.declined} declined`);
  if (row.interrupted > 0) parts.push(`${row.interrupted} interrupted`);
  if (row.calls > 0 && row.durationMs > 0) parts.push(`average ${duration(row.durationMs / row.calls)}`);
  if (row.longestMs > 0) parts.push(`longest ${duration(row.longestMs)}`);
  return parts.join(", ");
}

function ToolSection({ title, rows, noun }: { title: string; rows: readonly ToolRow[]; noun: string }) {
  if (rows.length === 0) return null;
  return (
    <Section title={title} aside={String(rows.reduce((total, row) => total + row.calls, 0))}>
      {rows.slice(0, LISTED).map((row) => (
        <Fact key={row.key} label={rowLabel(row)} title={`${rowLabel(row)}: ${useTitle(row)}`}>
          {row.calls}
          {row.failed > 0 && <span className="text-destructive"> · {row.failed} failed</span>}
        </Fact>
      ))}
      {rows.length > LISTED && <Quiet>and {plural(rows.length - LISTED, `more ${noun}`)}</Quiet>}
    </Section>
  );
}

/** MCP calls by server, then the tools most used. */
function McpSection({ rows }: { rows: readonly ToolRow[] }) {
  const servers = byServer(rows);
  if (servers.length === 0) return null;
  const tools = ofKind(rows, "mcp");
  return (
    <Section title="MCP calls" aside={String(servers.reduce((total, server) => total + server.calls, 0))}>
      {servers.map((server) => (
        <Fact key={server.server} label={server.server} title={`${plural(server.tools, "tool")} used`}>
          {server.calls}
          {server.failed > 0 && <span className="text-destructive"> · {server.failed} failed</span>}
        </Fact>
      ))}
      <ul className="flex flex-col gap-0.5 pl-3 text-xs text-muted-foreground">
        {tools.slice(0, LISTED).map((row) => (
          <li key={row.key} className="flex justify-between gap-3" title={useTitle(row)}>
            <span className="min-w-0 truncate">{rowLabel(row)}</span>
            <span className="tabular-nums">{row.calls}</span>
          </li>
        ))}
      </ul>
    </Section>
  );
}
