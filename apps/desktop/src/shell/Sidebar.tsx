import { FolderPlusIcon, PlusIcon, Settings2Icon } from "lucide-react";
import { useMemo, useState } from "react";

import { activityText, attentionLabel, attentionOf, findProvider, threadGroups, threadTree } from "../state/selectors";
import { store, useApp } from "../state/store";
import { NEW_THREAD, openNewThread } from "../modules/new-thread";
import { openProject } from "../modules/projects";
import { useUi } from "../ui/registry";
import { baseName, relativeTime } from "../util/format";
import { useNow } from "../util/useNow";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";

export function Sidebar() {
  const newThread = useUi((snapshot) => snapshot.commands.find((command) => command.id === NEW_THREAD));
  const threads = useApp((state) => state.threads);
  const plugins = useApp((state) => state.allPlugins);
  const projects = useApp((state) => state.projects);
  const selected = useApp((state) => state.selected);
  const connected = useApp((state) => state.daemon.status === "connected");
  const views = useUi((snapshot) => snapshot.views);
  const viewId = useApp((state) => state.view) ?? views[0]?.id;
  const [showArchived, setShowArchived] = useState(false);
  const now = useNow(30_000);

  const all = useMemo(() => Object.values(threads), [threads]);
  const archivedCount = all.filter((thread) => thread.archived).length;
  const groups = useMemo(
    () => threadGroups(all.filter((thread) => showArchived || !thread.archived), threads, projects),
    [all, threads, projects, showArchived],
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
        <div className="flex items-center justify-between pr-1 pl-3">
          <span className="text-xs text-muted-foreground">{projects.length === 0 ? "Threads by folder" : "Projects"}</span>
          <Button
            variant="ghost"
            size="xs"
            disabled={!connected}
            onClick={() => openProject()}
            title="Group repositories and folders under one name"
          >
            <PlusIcon /> Project
          </Button>
        </div>
        {groups.length === 0 && <p className="hint">No threads yet.</p>}
        {groups.map((group) => (
          <section key={group.key} className="group/section">
            <div className="flex items-center gap-1 pr-1">
              <h2 className="min-w-0 flex-1 truncate" title={group.kind === "project" ? group.project.folders.join("\n") : `${group.folder}\nNot in a project`}>
                {group.kind === "project" ? group.project.name : baseName(group.folder)}
              </h2>
              {group.kind === "project" ? (
                <>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    disabled={!connected}
                    aria-label={`New thread in ${group.project.name}`}
                    title="New thread in this project"
                    onClick={() => openNewThread({ project: group.project.id })}
                  >
                    <PlusIcon />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    disabled={!connected}
                    aria-label={`Edit ${group.project.name}`}
                    title="Name and folders"
                    onClick={() => openProject(group.project)}
                  >
                    <Settings2Icon />
                  </Button>
                </>
              ) : (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  disabled={!connected}
                  className="opacity-0 group-hover/section:opacity-100 focus-visible:opacity-100"
                  aria-label={`Make ${baseName(group.folder)} a project`}
                  title="Make this folder a project"
                  onClick={() => openProject(undefined, { name: baseName(group.folder), folders: [group.folder] })}
                >
                  <FolderPlusIcon />
                </Button>
              )}
            </div>
            {group.kind === "project" && group.threads.length === 0 && <p className="hint">No threads yet.</p>}
            {threadTree(group.threads).map(({ thread, depth, children }) => {
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
