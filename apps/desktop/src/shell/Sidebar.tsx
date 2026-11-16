import { ArrowLeftIcon, ArrowRightIcon, ChevronDownIcon, ChevronRightIcon, PlusIcon } from "lucide-react";
import { useMemo, useState } from "react";

import {
  activityText,
  attentionLabel,
  attentionOf,
  providerName,
  threadGroups,
  threadTitle,
  threadTree,
  type TreeRow,
} from "../state/selectors";
import { SIDEBAR, THREAD_MIN } from "../state/layout";
import { isUnread } from "../state/reads";
import { store, useApp } from "../state/store";
import { useUi } from "../ui/registry";
import { ResizeHandle, ThreadMenu } from "../ui/kit";
import { baseName, plural, relativeTime } from "../util/format";
import { useNow } from "../util/useNow";
import { type DragState, useThreadDrag } from "./useThreadDrag";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";

export function Sidebar() {
  // The shell offers what modules contribute, and nothing when they are left out.
  const newThread = useUi((snapshot) => snapshot.commands.find((command) => command.id === "thread.new"));
  const newProject = useUi((snapshot) => snapshot.commands.find((command) => command.id === "project.new"));
  const groupActions = useUi((snapshot) => snapshot.groupActions);
  const threads = useApp((state) => state.threads);
  const plugins = useApp((state) => state.allPlugins);
  const projects = useApp((state) => state.projects);
  const history = useApp((state) => state.history);
  const seen = useApp((state) => state.seen);
  const selected = useApp((state) => state.selected);
  const collapsed = useApp((state) => state.collapsed);
  const connected = useApp((state) => state.daemon.status === "connected");
  const width = useApp((state) => state.sidebarWidth);
  const views = useUi((snapshot) => snapshot.views);
  const viewId = useApp((state) => state.view) ?? views[0]?.id;
  const [showArchived, setShowArchived] = useState(false);
  const now = useNow(30_000);
  const { drag, start, wasDragged } = useThreadDrag();

  // A plugin's own threads are reached through the plugin, not listed.
  const all = useMemo(() => Object.values(threads).filter((thread) => !thread.hidden), [threads]);
  const archivedCount = all.filter((thread) => thread.archived).length;
  const groups = useMemo(
    () => threadGroups(all.filter((thread) => showArchived || !thread.archived), threads, projects),
    [all, threads, projects, showArchived],
  );

  return (
    <aside className="sidebar relative">
      <div className="sidebar-head">
        <span className="brand">qcc</span>
        <span className="ml-2 flex gap-0.5">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Back"
            title="Back (Alt+Left)"
            disabled={history.index === 0}
            onClick={() => store().navigate(-1)}
          >
            <ArrowLeftIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Forward"
            title="Forward (Alt+Right)"
            disabled={history.index >= history.places.length - 1}
            onClick={() => store().navigate(1)}
          >
            <ArrowRightIcon />
          </Button>
        </span>
        <span className="flex-1" />
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
          {newProject && (
            <Button
              variant="ghost"
              size="xs"
              disabled={!connected}
              onClick={newProject.run}
              title="Group repositories and folders under one name"
            >
              <PlusIcon /> Project
            </Button>
          )}
        </div>
        {groups.length === 0 && <p className="hint">No threads yet.</p>}
        {groups.map((group) => (
          <section key={group.key} className="group/section">
            <div
              data-group-top
              data-group={group.key}
              className={`flex items-center gap-1 rounded-md pr-1${
                drag?.over?.target === null && drag.drop && "parent" in drag.drop ? " bg-primary/15" : ""
              }`}
            >
              <h2 className="min-w-0 flex-1 truncate" title={group.kind === "project" ? group.project.folders.join("\n") : `${group.folder}\nNot in a project`}>
                {group.kind === "project" ? group.project.name : baseName(group.folder)}
              </h2>
              {groupActions
                .filter((action) => action.when?.(group) ?? true)
                .map((action) => (
                  <Button
                    key={action.id}
                    variant="ghost"
                    size="icon-xs"
                    disabled={!connected}
                    className={
                      action.onHover ? "opacity-0 group-hover/section:opacity-100 focus-visible:opacity-100" : undefined
                    }
                    aria-label={action.label(group)}
                    title={action.title}
                    onClick={() => action.run(group)}
                  >
                    <action.icon />
                  </Button>
                ))}
            </div>
            {group.kind === "project" && group.threads.length === 0 && <p className="hint">No threads yet.</p>}
            {threadTree(group.threads, collapsed, selected).map((row) => {
              const { thread, depth, children } = row;
              const attention = attentionOf(thread);
              const unread = isUnread(thread, seen);
              const provider = providerName(plugins, thread.provider);
              const indent = Math.min(depth, MAX_INDENT);
              const over = drag?.over?.target === thread.id ? drag.over : null;
              const landing = over && drag?.drop && "parent" in drag.drop ? over.zone : null;
              return (
                <div
                  key={thread.id}
                  data-thread-id={thread.id}
                  data-group={group.key}
                  className={`relative touch-none select-none${drag?.id === thread.id ? " opacity-40" : ""}${
                    landing === "into" ? " rounded-md bg-primary/15 outline outline-1 outline-primary" : ""
                  }`}
                  onPointerDown={(event) => start(event, thread, group.threads, group.key)}
                >
                  {landing === "beside" && (
                    // Where it lands among the threads next to this one.
                    <span
                      aria-hidden
                      className="pointer-events-none absolute right-1 bottom-0 z-10 h-0.5 rounded bg-primary"
                      style={{ left: `${8 + indent * INDENT}px` }}
                    />
                  )}
                  {/* A line down from each ancestor's dot ties a family together. */}
                  {Array.from({ length: indent }, (_, level) => (
                    <span
                      key={level}
                      aria-hidden
                      className="pointer-events-none absolute inset-y-0 border-l border-muted-foreground/30"
                      style={{ left: `${GUIDE_X + level * INDENT}px` }}
                    />
                  ))}
                  {indent > 0 && (
                    // The elbow from the parent's line to this thread's dot.
                    <span
                      aria-hidden
                      className="pointer-events-none absolute border-t border-muted-foreground/30"
                      style={{ left: `${GUIDE_X + (indent - 1) * INDENT}px`, top: `${DOT_Y}px`, width: `${INDENT - 6}px` }}
                    />
                  )}
                  <ThreadMenu
                    thread={thread}
                    render={
                      <button
                        className={`thread-row${thread.id === selected ? " selected" : ""}${thread.archived ? " archived" : ""}${depth > 0 ? " child" : ""}`}
                        style={depth > 0 ? { paddingLeft: `${8 + indent * INDENT}px` } : undefined}
                        onClick={() => {
                          if (!wasDragged()) store().select(thread.id);
                        }}
                      />
                    }
                  >
                    <span className={`dot ${attention}`} title={attentionLabel[attention]} />
                    <span
                      className={`thread-title${unread ? " font-semibold" : ""}${depth > 0 ? " text-[13px]" : ""}${depth > 0 && !unread && thread.id !== selected ? " text-foreground/80" : ""}`}
                    >
                      {threadTitle(thread)}
                    </span>
                    <span className="thread-time">{relativeTime(thread.updated_at, now)}</span>
                    <span className={`thread-meta${children > 0 ? " mr-11" : ""}`}>
                      {attention === "waiting"
                        ? "Needs your answer"
                        : unread && attention === "idle"
                          ? "New reply"
                        : attention === "failed"
                          ? "Failed"
                          : attention === "active" && thread.activity
                            ? activityText(thread)
                            : `${provider}${thread.environment ? " · worktree" : ""}`}
                    </span>
                  </ThreadMenu>
                  {children > 0 && <FamilyToggle row={row} />}
                </div>
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
      {drag && <DragGhost drag={drag} />}
      <ResizeHandle
        edge="right"
        width={width}
        bounds={SIDEBAR}
        room={() => window.innerWidth - THREAD_MIN}
        resize={store().setSidebarWidth}
        label="Thread list width"
      />
    </aside>
  );
}

/** Indentation per level of delegation, and how deep it goes before it stops. */
const INDENT = 14;
const MAX_INDENT = 4;
/** Where the middle of a root's dot is: its padding, its border, half the dot. */
const GUIDE_X = 8 + 1 + 4 - 0.5;
/** How far down a row the middle of its dot is: border, padding, half a line. */
const DOT_Y = 1 + 6 + 10;

/**
 * Folds or unfolds the threads a thread started. Folded, it counts every hidden
 * thread and shows the most pressing state among them, so a child asking for an
 * answer is not lost under a collapsed parent.
 */
function FamilyToggle({ row }: { row: TreeRow }) {
  const { thread, children, collapsed, hidden, hiddenAttention } = row;
  const Chevron = collapsed ? ChevronRightIcon : ChevronDownIcon;
  const count = collapsed ? hidden : children;
  const label = `${collapsed ? "Show" : "Hide"} ${plural(count, "thread")} it started`;
  return (
    <Button
      variant="ghost"
      size="xs"
      className="absolute right-1 bottom-1 h-5 gap-1 px-1 text-muted-foreground tabular-nums"
      aria-label={label}
      aria-expanded={!collapsed}
      title={hiddenAttention && hiddenAttention !== "idle" ? `${label}\nAmong them: ${attentionLabel[hiddenAttention].toLowerCase()}` : label}
      // A press on the toggle is not the start of dragging its row.
      onPointerDown={(event) => event.stopPropagation()}
      onClick={() => store().toggleCollapsed(thread.id)}
    >
      <Chevron />
      {collapsed && hiddenAttention && hiddenAttention !== "idle" && <span className={`dot ${hiddenAttention}`} />}
      {count}
    </Button>
  );
}

/** What follows the pointer while a thread is dragged: its name and what letting go does. */
function DragGhost({ drag }: { drag: DragState }) {
  const { drop } = drag;
  const what = !drop
    ? "Drop on a thread to put it under that one, between threads to move it next to them"
    : "refused" in drop
      ? drop.refused
      : drop.parent === null
        ? "Make it a thread of its own"
        : "Put it under this thread";
  return (
    <div
      className="pointer-events-none fixed z-50 max-w-72 rounded-md border border-border bg-popover px-2 py-1 text-xs shadow-md"
      style={{ left: drag.x + 14, top: drag.y + 10 }}
    >
      <div className="truncate font-medium text-foreground">{drag.title}</div>
      <div className={drop && "refused" in drop ? "text-destructive" : "text-muted-foreground"}>{what}</div>
    </div>
  );
}
