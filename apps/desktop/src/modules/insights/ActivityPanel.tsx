import { useMemo } from "react";

import { activityOf, turnSummary } from "../../state/stats";
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
      {activity.tools.length > 0 && (
        <Section title="Tools">
          {activity.tools.slice(0, LISTED).map(({ name, count }) => (
            <Fact key={name} label={name} title={name}>
              {count}
            </Fact>
          ))}
          {activity.tools.length > LISTED && (
            <Quiet>and {plural(activity.tools.length - LISTED, "more tool")}</Quiet>
          )}
        </Section>
      )}
    </>
  );
}
