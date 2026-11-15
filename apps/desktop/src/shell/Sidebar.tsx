import { useMemo, useState } from "react";

import type { Thread } from "../api/protocol.gen";
import { activityText, attentionLabel, attentionOf, findProvider, projectOf } from "../state/selectors";
import { store, useApp } from "../state/store";
import { NEW_THREAD } from "../modules/new-thread";
import { useUi } from "../ui/registry";
import { baseName, relativeTime } from "../util/format";
import { useNow } from "../util/useNow";

type Group = { project: string; threads: Thread[]; latest: number };

function groupByProject(threads: Thread[]): Group[] {
  const groups = new Map<string, Group>();
  for (const thread of threads) {
    const project = projectOf(thread);
    const group = groups.get(project) ?? { project, threads: [], latest: 0 };
    group.threads.push(thread);
    group.latest = Math.max(group.latest, thread.updated_at);
    groups.set(project, group);
  }
  for (const group of groups.values()) group.threads.sort((a, b) => b.created_at - a.created_at);
  return [...groups.values()].sort((a, b) => b.latest - a.latest);
}

export function Sidebar() {
  const newThread = useUi((snapshot) => snapshot.commands.find((command) => command.id === NEW_THREAD));
  const threads = useApp((state) => state.threads);
  const plugins = useApp((state) => state.plugins);
  const selected = useApp((state) => state.selected);
  const connected = useApp((state) => state.daemon.status === "connected");
  const views = useUi((snapshot) => snapshot.views);
  const viewId = useApp((state) => state.view) ?? views[0]?.id;
  const [showArchived, setShowArchived] = useState(false);
  const now = useNow(30_000);

  const all = useMemo(() => Object.values(threads), [threads]);
  const archivedCount = all.filter((thread) => thread.archived).length;
  const groups = useMemo(
    () => groupByProject(all.filter((thread) => showArchived || !thread.archived)),
    [all, showArchived],
  );

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <span className="brand">qcc</span>
        {newThread && (
          <button className="primary small" disabled={!connected} onClick={newThread.run} title={newThread.shortcut}>
            {newThread.title}
          </button>
        )}
      </div>
      <nav className="view-list" aria-label="Screens">
        {views.map((view) => (
          <button
            key={view.id}
            className={`view-row${selected === null && view.id === viewId ? " selected" : ""}`}
            onClick={() => store().openView(view.id)}
          >
            {view.title}
          </button>
        ))}
      </nav>
      <nav className="thread-list" aria-label="Threads">
        {groups.length === 0 && <p className="hint">No threads yet.</p>}
        {groups.map((group) => (
          <section key={group.project}>
            <h2 title={group.project}>{baseName(group.project)}</h2>
            {group.threads.map((thread) => {
              const attention = attentionOf(thread);
              const provider = findProvider(plugins, thread.provider)?.name ?? thread.provider;
              return (
                <button
                  key={thread.id}
                  className={`thread-row${thread.id === selected ? " selected" : ""}${thread.archived ? " archived" : ""}`}
                  onClick={() => store().select(thread.id)}
                >
                  <span className={`dot ${attention}`} title={attentionLabel[attention]} />
                  <span className="thread-title">{thread.title ?? "New thread"}</span>
                  <span className="thread-time">{relativeTime(thread.updated_at, now)}</span>
                  <span className="thread-meta">
                    {attention === "waiting"
                      ? "Needs your answer"
                      : attention === "failed"
                        ? "Failed"
                        : attention === "active" && thread.activity
                          ? activityText(thread)
                          : `${provider}${thread.environment ? " · worktree" : ""}`}
                  </span>
                </button>
              );
            })}
          </section>
        ))}
      </nav>
      {archivedCount > 0 && (
        <label className="sidebar-foot">
          <input type="checkbox" checked={showArchived} onChange={(event) => setShowArchived(event.target.checked)} />
          Show archived ({archivedCount})
        </label>
      )}
    </aside>
  );
}
