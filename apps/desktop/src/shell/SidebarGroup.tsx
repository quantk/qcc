// One group of the thread list: a project, or a folder of threads in no project.
// Its heading folds it; folded, the heading tells what the hidden threads need.

import { ChevronRightIcon, FolderClosedIcon, FolderKanbanIcon, FolderOpenIcon } from "lucide-react";
import type { KeyboardEvent, ReactNode } from "react";

import { type GroupSummary, summaryText } from "../state/groups";
import type { ThreadGroup } from "../state/selectors";
import type { GroupAction } from "../ui/registry";
import { baseName } from "../util/format";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

export function GroupSection({
  group,
  folded,
  summary,
  actions,
  connected,
  dropOnTop,
  onFold,
  peek,
  children,
}: {
  group: ThreadGroup;
  folded: boolean;
  /** What the hidden threads need; only a folded group has one. */
  summary: GroupSummary | null;
  actions: readonly GroupAction[];
  connected: boolean;
  /** A thread dragged over the heading would become a thread of its own. */
  dropOnTop: boolean;
  /** Fold this group, or with `all` every group, or unfold. */
  onFold: (fold: boolean, all: boolean) => void;
  /** What stays in sight while folded: the selected thread. */
  peek: ReactNode;
  children: ReactNode;
}) {
  const name = group.kind === "project" ? group.project.name : baseName(group.folder);
  const Icon = group.kind === "project" ? FolderKanbanIcon : folded ? FolderClosedIcon : FolderOpenIcon;

  // Like a tree view: Left folds the group or leaves a row for its heading, Right
  // unfolds it or goes to its first row.
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const section = event.currentTarget;
    const target = event.target as HTMLElement;
    const heading = section.querySelector<HTMLElement>("[data-group-heading]");
    if (target === heading) {
      if (event.key === "ArrowLeft" && !folded) onFold(true, false);
      else if (event.key === "ArrowRight" && folded) onFold(false, false);
      else if (event.key === "ArrowRight") section.querySelector<HTMLElement>(".thread-row")?.focus();
      else return;
    } else if (event.key === "ArrowLeft" && target.classList.contains("thread-row")) {
      heading?.focus();
    } else return;
    event.preventDefault();
  };

  return (
    <Collapsible
      render={<section />}
      open={!folded}
      onOpenChange={(open, details) => onFold(!open, (details.event as MouseEvent | undefined)?.altKey ?? false)}
      className="group/section not-first:mt-1 not-first:border-t not-first:border-border/70 not-first:pt-1"
      onKeyDown={onKeyDown}
    >
      <div
        data-group-top
        data-group={group.key}
        // Sticky, the heading of the group in view stays at the top while its threads scroll.
        className={cn(
          "sticky top-0 z-10 flex items-center gap-0.5 rounded-md bg-secondary pr-1",
          dropOnTop && "bg-primary/15",
        )}
      >
        <h2 className="min-w-0 flex-1">
          <CollapsibleTrigger
            data-group-heading
            className={cn(
              "flex w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left text-[13px] font-semibold text-foreground",
              "outline-none hover:bg-card/70 focus-visible:ring-2 focus-visible:ring-ring/50",
              group.kind === "folder" && "font-medium text-foreground/80",
            )}
            title={`${group.kind === "project" ? group.project.folders.join("\n") : `${group.folder}\nNot in a project`}${
              summary ? `\n${summaryText(summary)}` : ""
            }\nAlt+click: fold or unfold every group`}
          >
            <ChevronRightIcon
              aria-hidden
              className={cn(
                "size-3.5 flex-none text-muted-foreground transition-transform duration-150 motion-reduce:transition-none",
                !folded && "rotate-90",
              )}
            />
            <Icon aria-hidden className="size-3.5 flex-none text-muted-foreground" />
            <span className="min-w-[4ch] truncate">{name}</span>
            {summary && <Summary summary={summary} />}
          </CollapsibleTrigger>
        </h2>
        {actions
          .filter((action) => action.when?.(group) ?? true)
          .map((action) => (
            <Button
              key={action.id}
              variant="ghost"
              size="icon-xs"
              disabled={!connected}
              // Folded, the summary needs the room: every action waits for the pointer.
              className={cn(
                folded
                  ? "hidden group-has-[:focus-visible]/section:inline-flex group-hover/section:inline-flex"
                  : action.onHover && "opacity-0 group-hover/section:opacity-100 focus-visible:opacity-100",
              )}
              aria-label={action.label(group)}
              title={action.title}
              onClick={() => action.run(group)}
            >
              <action.icon />
            </Button>
          ))}
      </div>
      <CollapsibleContent
        className={cn(
          "h-(--collapsible-panel-height) overflow-hidden transition-[height] duration-150 ease-out",
          "data-ending-style:h-0 data-starting-style:h-0 motion-reduce:transition-none",
        )}
      >
        <div className="pt-0.5 pl-2">{children}</div>
      </CollapsibleContent>
      {peek && <div className="pl-2">{peek}</div>}
    </Collapsible>
  );
}

/**
 * The hidden threads at a glance: how many wait for an answer, failed, work or have
 * a reply not yet read, then how many there are. Waiting comes first and stands out: an agent is stopped
 * until the user answers it.
 */
function Summary({ summary }: { summary: GroupSummary }) {
  return (
    <span aria-label={summaryText(summary)} className="ml-auto flex flex-none items-center gap-1 pl-1 text-[11px] font-medium tabular-nums">
      {summary.waiting > 0 && (
        <Pill className="bg-(--blue) text-background" dot="waiting" count={summary.waiting} inverted />
      )}
      {summary.failed > 0 && <Pill className="bg-(--red)/12 text-(--red)" dot="failed" count={summary.failed} />}
      {summary.active > 0 && <Pill className="bg-(--brand)/12 text-(--brand)" dot="active" count={summary.active} />}
      {summary.unread > 0 && (
        <Pill className="bg-foreground/10 text-foreground" dot="border-foreground bg-foreground" count={summary.unread} />
      )}
      {/* The count gives way to the heading's buttons when they show. */}
      <span className="min-w-4 text-right text-muted-foreground group-has-[:focus-visible]/section:hidden group-hover/section:hidden">
        {summary.threads}
      </span>
    </span>
  );
}

function Pill({
  className,
  dot,
  count,
  inverted = false,
}: {
  className: string;
  dot: string;
  count: number;
  inverted?: boolean;
}) {
  return (
    <span aria-hidden className={cn("inline-flex h-4 items-center gap-1 rounded-full px-1.5", className)}>
      <span className={cn("dot size-1.5", dot, inverted && "border-background bg-background")} />
      {count}
    </span>
  );
}
