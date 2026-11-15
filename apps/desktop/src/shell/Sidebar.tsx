import { useMemo, useState } from "react";

import type { Thread } from "../api/protocol.gen";
import {
  type TreeRow,
  activityText,
  attentionLabel,
  attentionOf,
  findProvider,
  projectOf,
  rootOf,
  threadTree,
} from "../state/selectors";
import { store, useApp } from "../state/store";
import { NEW_THREAD } from "../modules/new-thread";
import { useUi } from "../ui/registry";
import { baseName, relativeTime } from "../util/format";
import { useNow } from "../util/useNow";
import { PlusIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";

type Group = { project: string; rows: TreeRow[]; latest: number };

/**
 * Threads by project, delegated ones under the thread that started them. A family
 * stays together under the project of the thread the user began, wherever its
 * members work.
 */
function groupByProject(threads: Thread[], all: Readonly<Record<string, Thread>>): Group[] {
  const members = new Map<string, Thread[]>();
  const latest = new Map<string, number>();
  for (const thread of threads) {
    const project = projectOf(rootOf(thread, all));
    members.set(project, [...(members.get(project) ?? []), thread]);
    latest.set(project, Math.max(latest.get(project) ?? 0, thread.updated_at));
  }
  return [...members.entries()]
    .map(([project, threads]) => ({ project, rows: threadTree(threads), latest: latest.get(project) ?? 0 }))
    .sort((a, b) => b.latest - a.latest);
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
    () => groupByProject(all.filter((thread) => showArchived || !thread.archived), threads),
    [all, threads, showArchived],
  );

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <span className="brand">qcc</span>
        {newThread && (
          <Button size="sm" disabled={!connected} onClick={newThread.run} title={newThread.shortcut}>
            <PlusIcon /> {newThread.title}
          </Button>
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
            {group.rows.map(({ thread, depth, children }) => {
              const attention = attentionOf(thread);
              const provider = findProvider(plugins, thread.provider)?.name ?? thread.provider;
              return (
                <button
                  key={thread.id}
                  className={`thread-row${thread.id === selected ? " selected" : ""}${thread.archived ? " archived" : ""}${depth > 0 ? " child" : ""}`}
                  style={depth > 0 ? { paddingLeft: `${8 + Math.min(depth, 4) * 14}px` } : undefined}
                  title={children > 0 ? `Started ${children} other thread${children === 1 ? "" : "s"}` : undefined}
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
          <Checkbox checked={showArchived} onCheckedChange={(checked) => setShowArchived(checked)} />
          Show archived ({archivedCount})
        </label>
      )}
    </aside>
  );
}
