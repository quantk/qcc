import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import {
  type Attention,
  activityText,
  attentionLabel,
  attentionOf,
  findProvider,
  folderOf,
} from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { InteractionPrompt } from "../../ui/kit";
import { baseName, elapsed, plural, relativeTime } from "../../util/format";
import { useNow } from "../../util/useNow";

type Lane = { id: string; title: string; hint: string; shows: Attention[] };

// Ordered by how much each lane wants from the user.
const LANES: Lane[] = [
  { id: "needs-you", title: "Needs you", hint: "Waiting for an answer, or failed", shows: ["waiting", "failed"] },
  { id: "working", title: "Working", hint: "A turn is running", shows: ["active"] },
  { id: "idle", title: "Idle", hint: "Finished their last turn", shows: ["idle"] },
];

function AgentCard({ thread, now }: { thread: Thread; now: number }) {
  const plugins = useApp((state) => state.allPlugins);
  const attention = attentionOf(thread);
  const provider = findProvider(plugins, thread.provider)?.name ?? thread.provider;
  const parent = useApp((state) => (thread.parent ? state.threads[thread.parent] : undefined));
  const project = useApp((state) => state.projects.find((candidate) => candidate.id === thread.project));
  const activity = activityText(thread);
  const technical = thread.activity && !["agentMessage", "reasoning", "plan"].includes(thread.activity.kind);
  const open = () => store().select(thread.id);

  return (
    <article
      className={`agent-card ${attention}`}
      role="button"
      tabIndex={0}
      onClick={open}
      onKeyDown={(event) => {
        if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) open();
      }}
    >
      <header>
        <span className={`dot ${attention}`} title={attentionLabel[attention]} />
        <h3>{thread.title ?? "New thread"}</h3>
        <span className="agent-time">
          {attention === "active" && thread.turn_started_at
            ? elapsed(now - thread.turn_started_at)
            : relativeTime(thread.updated_at, now)}
        </span>
      </header>
      <div className="agent-facts">
        <span title={folderOf(thread)}>{project ? project.name : baseName(folderOf(thread))}</span>
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
      {attention === "failed" && <div className="agent-failed">{thread.status === "failed" ? "The session failed" : "The last turn failed"} — open the thread to see why</div>}
      {activity && <div className={`agent-activity${technical ? " technical" : ""}`}>{activity}</div>}
      {(thread.pending_interactions ?? []).map((pending) => (
        <InteractionPrompt key={pending.id} threadId={thread.id} id={pending.id} request={pending.request} />
      ))}
    </article>
  );
}

/** The control panel: every agent at once, sorted by what it wants from you. */
export function Overview() {
  const threads = useApp((state) => state.threads);
  const live = useMemo(() => Object.values(threads).filter((thread) => !thread.archived), [threads]);
  const anyWorking = live.some((thread) => thread.status === "active");
  const now = useNow(anyWorking ? 1000 : 30_000);

  const lanes = LANES.map((lane) => ({
    ...lane,
    threads: live
      .filter((thread) => lane.shows.includes(attentionOf(thread)))
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
