// Which part of the thread's work the diff shows: all of it, the uncommitted work,
// one commit, or a run of them (shift-click), the uncommitted work possibly among them.

import { ChevronDownIcon, GitCommitHorizontalIcon, PencilIcon } from "lucide-react";
import { type MouseEvent, useRef, useState } from "react";

import type { ChangeScope, Revision } from "../../api/protocol.gen";
import type { ChangesState } from "../../state/changes";
import { ALL, scopeLabel, scopeOfSpan, shortId, spanBetween, spanOfScope } from "../../state/changeScope";
import { relativeTime } from "../../util/format";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * Whether there is anything to pick: without commits since the base, all the work
 * is the uncommitted work.
 */
export function picking(state: ChangesState): boolean {
  return state.revisions.length > 0 || state.scope.kind !== "all";
}

/** What an empty list says, by what it was asked for. */
export function nothingChanged(state: ChangesState): string {
  if (state.scope.kind === "all") return "The agent has not changed any files.";
  if (state.scope.kind === "unsaved") return "Nothing is uncommitted.";
  return "These commits change no files.";
}

export function ScopePicker({
  scope,
  revisions,
  pick,
  size = "sm",
}: {
  scope: ChangeScope;
  revisions: readonly Revision[];
  pick: (scope: ChangeScope) => void;
  size?: "sm" | "xs";
}) {
  const [open, setOpen] = useState(false);
  const span = spanOfScope(scope, revisions);
  // The row a shift-click extends from: the last one clicked, or the newest shown.
  const anchor = useRef<number | null>(null);
  const now = Date.now();

  const choose = (row: number, event: MouseEvent) => {
    const from = anchor.current ?? span?.newest ?? row;
    if (event.shiftKey) {
      pick(scopeOfSpan(spanBetween(from, row), revisions));
      return;
    }
    anchor.current = row;
    pick(scopeOfSpan({ newest: row, oldest: row }, revisions));
    setOpen(false);
  };
  const chosen = (row: number) => scope.kind !== "all" && span !== null && span.newest <= row && row <= span.oldest;
  const rowClass = (row: number) =>
    cn("h-auto w-full justify-start gap-2 px-2 py-1 text-left font-normal", chosen(row) && "bg-muted");

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button variant="outline" size={size} className="max-w-72 min-w-0" title="Which changes to show">
            <span className="truncate">{scopeLabel(scope, revisions)}</span>
            <ChevronDownIcon />
          </Button>
        }
      />
      <PopoverContent align="start" className="max-h-[min(480px,60vh)] w-96 max-w-[calc(100vw-2rem)] gap-0.5 overflow-y-auto p-1">
        <Button
          variant="ghost"
          size="sm"
          className={cn("h-auto w-full justify-start px-2 py-1 font-normal", scope.kind === "all" && "bg-muted")}
          onClick={() => {
            anchor.current = null;
            pick(ALL);
            setOpen(false);
          }}
        >
          All changes
          <span className="ml-auto text-xs text-muted-foreground">since the base</span>
        </Button>
        <Button variant="ghost" size="sm" className={rowClass(0)} onClick={(event) => choose(0, event)}>
          <PencilIcon className="text-muted-foreground" />
          Uncommitted
        </Button>
        {revisions.map((revision, index) => (
          <Button
            key={revision.id}
            variant="ghost"
            size="sm"
            className={rowClass(index + 1)}
            title={`${revision.id}\n${revision.title}`}
            onClick={(event) => choose(index + 1, event)}
          >
            <GitCommitHorizontalIcon className="text-muted-foreground" />
            <code className="flex-none text-xs text-muted-foreground">{shortId(revision.id)}</code>
            <span className="min-w-0 flex-1 truncate">{revision.title}</span>
            {revision.time != null && (
              <span className="flex-none text-xs text-muted-foreground">{relativeTime(revision.time, now)}</span>
            )}
          </Button>
        ))}
        <p className="px-2 pt-1.5 pb-1 text-xs text-muted-foreground">Shift-click to show a run of commits.</p>
      </PopoverContent>
    </Popover>
  );
}
