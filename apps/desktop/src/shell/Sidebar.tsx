import {
  AppWindowIcon,
  ArchiveIcon,
  ArrowLeftIcon,
  ArrowRightIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  PlusIcon,
} from "lucide-react";
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
import { BlockMenu, useSlot } from "../ui/blocks";
import { type AppBlock, type ViewBadge, useUi } from "../ui/registry";
import { isAppBlock, openViewCommand } from "../ui/views";
import { ResizeHandle, ThreadMenu } from "../ui/kit";
import { baseName, plural, relativeTime } from "../util/format";
import { useNow } from "../util/useNow";
import { SidebarBlocks } from "./SidebarBlocks";
import { type DragState, useThreadDrag } from "./useThreadDrag";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/ui/kbd";
import { Toggle } from "@/components/ui/toggle";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

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
  // The screens' buttons: the top row and the foot are two slots of the layout.
  const topSlot = useSlot("views");
  const footSlot = useSlot("views.foot");
  const top = useMemo(() => topSlot.filter(isAppBlock), [topSlot]);
  const bottom = useMemo(() => footSlot.filter(isAppBlock), [footSlot]);
  const viewId = useApp((state) => state.view) ?? (top[0] ?? bottom[0])?.id;
  // A view is current only while no thread covers it.
  const current = selected === null ? viewId : undefined;
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
    <TooltipProvider delay={300}>
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
        {top.length > 0 && (
          <nav className="flex flex-wrap items-center gap-1 border-b border-border px-3 pb-2" aria-label="Screens">
            {top.map((view) => (
              <ViewButton key={view.id} view={view} current={view.id === current} side="bottom" />
            ))}
          </nav>
        )}
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
        <SidebarBlocks />
        {(bottom.length > 0 || archivedCount > 0) && (
          // Setup and preferences sit at the foot, where they leave the threads their room.
          <div className="flex items-center gap-1 border-t border-border px-3 py-1.5">
            <nav className="flex flex-1 flex-wrap items-center gap-1" aria-label="Setup">
              {bottom.map((view) => (
                <ViewButton key={view.id} view={view} current={view.id === current} side="top" />
              ))}
            </nav>
            {archivedCount > 0 && (
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Toggle
                      size="sm"
                      className="gap-1 px-2 text-muted-foreground tabular-nums aria-pressed:bg-card aria-pressed:text-foreground"
                      aria-label={`Show archived threads (${archivedCount})`}
                      pressed={showArchived}
                      onPressedChange={setShowArchived}
                    />
                  }
                >
                  <ArchiveIcon />
                  {archivedCount}
                </TooltipTrigger>
                <TooltipContent side="top">{showArchived ? "Hide" : "Show"} archived threads</TooltipContent>
              </Tooltip>
            )}
          </div>
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
    </TooltipProvider>
  );
}

/**
 * The button of one view: its icon, with its name, its count and its shortcut in a
 * tooltip. A view that counts something reads its count here, so only its own
 * button redraws when the count changes.
 */
function ViewButton({ view, current, side }: { view: AppBlock; current: boolean; side: "top" | "bottom" }) {
  return view.useBadge ? (
    <CountedViewButton view={view} useBadge={view.useBadge} current={current} side={side} />
  ) : (
    <ViewButtonBody view={view} badge={null} current={current} side={side} />
  );
}

function CountedViewButton({
  useBadge,
  ...props
}: { view: AppBlock; useBadge: () => ViewBadge | null; current: boolean; side: "top" | "bottom" }) {
  return <ViewButtonBody {...props} badge={useBadge()} />;
}

function ViewButtonBody({
  view,
  badge,
  current,
  side,
}: {
  view: AppBlock;
  badge: ViewBadge | null;
  current: boolean;
  side: "top" | "bottom";
}) {
  const shortcut = useUi((snapshot) => snapshot.commands.find((command) => command.id === openViewCommand(view.id))?.shortcut);
  const Icon = view.icon ?? AppWindowIcon;
  // A right click on the button offers to move or hide its block, like a tab's.
  return (
    <BlockMenu block={view} render={<span className="contents" />}>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon"
              aria-label={badge ? `${view.title}: ${badge.label}` : view.title}
              aria-current={current ? "page" : undefined}
              className={cn(
                "relative text-muted-foreground hover:bg-card hover:text-foreground",
                current && "border-border bg-card text-foreground shadow-xs",
              )}
              onClick={() => store().openView(view.id)}
            />
          }
        >
          <Icon />
          {badge && badge.count > 0 && (
            <Badge
              aria-hidden
              className={cn(
                "pointer-events-none absolute -top-1 -right-1 h-4 min-w-4 px-1 text-[10px] tabular-nums ring-2 ring-secondary",
                badge.tone === "attention" && "bg-(--blue) text-background",
              )}
            >
              {badge.count > 99 ? "99+" : badge.count}
            </Badge>
          )}
        </TooltipTrigger>
        <TooltipContent side={side} className="flex-col items-start gap-0.5">
          <span className="flex items-center gap-2">
            {view.title}
            {shortcut && <Kbd>{shortcut}</Kbd>}
          </span>
          {badge && <span className="opacity-70">{badge.label}</span>}
        </TooltipContent>
      </Tooltip>
    </BlockMenu>
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
