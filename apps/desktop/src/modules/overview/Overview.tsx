import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import {
  type Attention,
  activityText,
  attentionLabel,
  attentionOf,
  cardTime,
  failureText,
  folderOf,
  providerName,
  threadTitle,
} from "../../state/selectors";
import { isUnread } from "../../state/reads";
import { store, useApp } from "../../state/store";
import { InteractionPrompt, ProjectIconView, ThreadMenu } from "../../ui/kit";
import { baseName, plural } from "../../util/format";
import { useNow } from "../../util/useNow";

type Lane = { id: string; title: string; hint: string; shows: Attention[]; unread?: boolean };

// Ordered by how much each lane wants from the user.
const LANES: Lane[] = [
  {
    id: "needs-you",
    title: "Needs you",
    hint: "Waiting for an answer, failed, or replied since you last looked",
    shows: ["waiting", "failed"],
    unread: true,
  },
  { id: "working", title: "Working", hint: "A turn is running", shows: ["active"] },
  { id: "idle", title: "Idle", hint: "Finished their last turn", shows: ["idle"] },
];

function AgentCard({ thread, now }: { thread: Thread; now: number }) {
  const plugins = useApp((state) => state.allPlugins);
  const attention = attentionOf(thread);
  const provider = providerName(plugins, thread.provider);
  const parent = useApp((state) => (thread.parent ? state.threads[thread.parent] : undefined));
  const project = useApp((state) => state.projects.find((candidate) => candidate.id === thread.project));
  const unread = useApp((state) => isUnread(thread, state.seen));
  const activity = activityText(thread);
  const technical = thread.activity && !["agentMessage", "reasoning", "plan"].includes(thread.activity.kind);
  const open = () => store().select(thread.id);

  return (
    <ThreadMenu
      thread={thread}
      render={
        <article
          className={`agent-card ${attention}`}
          role="button"
          tabIndex={0}
          onClick={open}
          onKeyDown={(event) => {
            if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) open();
          }}
        />
      }
    >
      <header>
        <span className={`dot ${attention}`} title={attentionLabel[attention]} />
        <h3>{threadTitle(thread)}</h3>
        <span className="agent-time">{cardTime(thread, now)}</span>
      </header>
      <div className="agent-facts">
        <span className="inline-flex min-w-0 items-center gap-1" title={folderOf(thread)}>
          {project?.icon && <ProjectIconView icon={project.icon} size={12} />}
          {project ? project.name : baseName(folderOf(thread))}
        </span>
        <span>{provider}</span>
        {thread.model && <span>{thread.model}</span>}
        {thread.environment && <span>worktree</span>}
        {parent && (
          <span className="agent-parent" title="Started by another agent">
            ↳ {parent.title ?? "a thread"}
          </span>
        )}
        {(thread.queued_inputs ?? 0) > 0 && <span>{plural(thread.queued_inputs ?? 0, "message")} queued</span>}
      </div>
      {unread && attention === "idle" && <div className="text-xs font-semibold text-(--blue)">New reply</div>}
      {attention === "failed" && <div className="agent-failed">{failureText(thread)} — open the thread to see why</div>}
      {activity && <div className={`agent-activity${technical ? " technical" : ""}`}>{activity}</div>}
      {(thread.pending_interactions ?? []).map((pending) => (
        <InteractionPrompt key={pending.id} threadId={thread.id} id={pending.id} request={pending.request} />
      ))}
    </ThreadMenu>
  );
}

/** The control panel: every agent at once, sorted by what it wants from you. */
export function Overview() {
  const threads = useApp((state) => state.threads);
  const live = useMemo(
    () => Object.values(threads).filter((thread) => !thread.archived && !thread.hidden),
    [threads],
  );
  const anyWorking = live.some((thread) => thread.status === "active");
  const now = useNow(anyWorking ? 1000 : 30_000);

  const seen = useApp((state) => state.seen);
  // A thread with an unread reply is in "Needs you", not also among the idle.
  const laneOf = (thread: Thread) => {
    const attention = attentionOf(thread);
    if (attention === "idle" && isUnread(thread, seen)) return "unread";
    return attention;
  };
  const lanes = LANES.map((lane) => ({
    ...lane,
    threads: live
      .filter((thread) => {
        const where = laneOf(thread);
        return where === "unread" ? Boolean(lane.unread) : lane.shows.includes(where);
      })
      .sort((a, b) => b.updated_at - a.updated_at),
  }));

  return (
    <div className="overview">
      <header className="overview-head">
        <h1>Overview</h1>
        <div className="overview-counts">
          {lanes.map((lane) => (
            <span key={lane.id} className={`count ${lane.id}`} title={lane.hint}>
              <strong>{lane.threads.length}</strong> {lane.title.toLowerCase()}
            </span>
          ))}
        </div>
      </header>
      {live.length === 0 ? (
        <div className="empty">
          <p>No agents yet.</p>
          <p className="keys">
            <kbd>Ctrl+N</kbd> starts a thread · <kbd>Ctrl+K</kbd> lists every command
          </p>
        </div>
      ) : (
        <div className="lanes">
          {lanes.map((lane) => (
            <section key={lane.id} className={`lane ${lane.id}`} aria-label={lane.title}>
              <h2>
                {lane.title} <span>{lane.threads.length}</span>
              </h2>
              {lane.threads.length === 0 && <p className="lane-empty">Nobody here.</p>}
              {lane.threads.map((thread) => (
                <AgentCard key={thread.id} thread={thread} now={now} />
              ))}
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
