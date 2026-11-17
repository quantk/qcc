import { ArchiveIcon, ArrowUpIcon, NetworkIcon, SearchIcon, SearchXIcon, XIcon } from "lucide-react";
import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import {
  CHILD_STATES,
  type ChildDepth,
  type ChildFilter,
  type ChildList,
  type ChildRow,
  type ChildSort,
  type ChildState,
  childList,
  childSortLabel,
  childStateLabel,
  clearNarrowing,
  isNarrowed,
  useChildFilter,
} from "../../state/children";
import {
  activityText,
  attentionLabel,
  attentionOf,
  cardTime,
  failureText,
  providerName,
  threadTitle,
} from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Choice, InteractionPrompt } from "../../ui/kit";
import { plural } from "../../util/format";
import { useNow } from "../../util/useNow";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Toggle } from "@/components/ui/toggle";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export function ParentLink({ parent }: { parent: Thread }) {
  return (
    <Button
      variant="outline"
      size="sm"
      className="parent-link"
      title={`Started by: ${parent.title ?? parent.id}`}
      onClick={() => store().select(parent.id)}
    >
      <ArrowUpIcon /> {parent.title ?? "Parent thread"}
    </Button>
  );
}

/** The id of the Agents block: the full view of a thread's children. */
export const AGENTS = "agents";

/** What a child is up to, in a line. */
export function childStatus(thread: Thread): string {
  const attention = attentionOf(thread);
  if (attention === "active") return activityText(thread) || "Working";
  if (attention === "waiting") return "Waiting for your answer";
  if (attention === "failed") return failureText(thread);
  if (thread.last_turn_status === "interrupted") return "Stopped";
  if (!thread.last_turn_status) return "Not started";
  return thread.activity?.kind === "agentMessage" && thread.activity.detail ? thread.activity.detail : "Done";
}

/** The dot each state shows, the one the sidebar's rows use. */
const STATE_DOT: Record<ChildState, string> = {
  waiting: "waiting",
  working: "active",
  failed: "failed",
  idle: "idle",
  done: "idle",
};

/**
 * The threads a thread started and theirs in turn, as a tree: what each is doing,
 * and the questions they ask. Archived ones are left out unless asked for.
 */
export function ChildrenView({ parent }: { parent: Thread }) {
  const threads = useApp((state) => state.threads);
  const [filter, change] = useChildFilter(parent.id);
  const all = useMemo(() => Object.values(threads), [threads]);
  const list = useMemo(() => childList(all, parent.id, filter), [all, parent.id, filter]);
  const narrowed = isNarrowed(filter);

  return (
    <TooltipProvider delay={300}>
      <div className="children">
        <Toolbar filter={filter} list={list} change={change} />
        {list.rows.length > 0 ? (
          <>
            {(narrowed || list.archived > 0) && (
              <p className="text-xs text-muted-foreground">
                {narrowed ? `${list.shown} of ` : ""}
                {plural(list.total - (filter.archived ? 0 : list.archived), "agent")}
                {!filter.archived && list.archived > 0 ? `, ${list.archived} archived hidden` : ""}
              </p>
            )}
            {list.rows.map((row) =>
              row.match ? (
                <ChildCard key={row.thread.id} row={row} nestedShown={filter.depth === "all"} />
              ) : (
                <PathRow key={row.thread.id} row={row} />
              ),
            )}
          </>
        ) : (
          <EmptyList filter={filter} list={list} change={change} />
        )}
      </div>
    </TooltipProvider>
  );
}

function Toolbar({
  filter,
  list,
  change,
}: {
  filter: ChildFilter;
  list: ChildList;
  change: (change: Partial<ChildFilter>) => void;
}) {
  const plugins = useApp((state) => state.allPlugins);
  // Options nobody would match are left out, unless chosen: then they stay to be unchosen.
  const states = CHILD_STATES.filter((state) => list.states[state] > 0 || filter.states.includes(state));
  const providers = [...new Set([...list.providers.keys(), ...filter.providers])].sort();
  if (list.total === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-2" role="toolbar" aria-label="Filter agents">
      <InputGroup className="h-7 w-48">
        <InputGroupAddon>
          <SearchIcon />
        </InputGroupAddon>
        <InputGroupInput
          placeholder="Search by title"
          aria-label="Search agents by title"
          value={filter.query}
          onChange={(event) => change({ query: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === "Escape" && filter.query) {
              event.stopPropagation();
              change({ query: "" });
            }
          }}
        />
        {filter.query && (
          <InputGroupAddon align="inline-end">
            <InputGroupButton size="icon-xs" aria-label="Clear the search" onClick={() => change({ query: "" })}>
              <XIcon />
            </InputGroupButton>
          </InputGroupAddon>
        )}
      </InputGroup>
      {states.length > 1 || filter.states.length > 0 ? (
        <ToggleGroup
          multiple
          size="sm"
          variant="outline"
          spacing={0}
          aria-label="Status"
          value={[...filter.states]}
          onValueChange={(values: unknown[]) => change({ states: values as ChildState[] })}
        >
          {states.map((state) => (
            <ToggleGroupItem
              key={state}
              value={state}
              className="gap-1.5 tabular-nums data-pressed:bg-accent data-pressed:text-foreground"
            >
              <span className={`dot ${STATE_DOT[state]}`} />
              {childStateLabel[state]}
              <span className="text-muted-foreground">{list.states[state]}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      ) : null}
      {providers.length > 1 || filter.providers.length > 0 ? (
        <ToggleGroup
          multiple
          size="sm"
          variant="outline"
          spacing={0}
          aria-label="Agent"
          value={[...filter.providers]}
          onValueChange={(values: unknown[]) => change({ providers: values as string[] })}
        >
          {providers.map((provider) => (
            <ToggleGroupItem
              key={provider}
              value={provider}
              className="gap-1.5 tabular-nums data-pressed:bg-accent data-pressed:text-foreground"
            >
              {providerName(plugins, provider)}
              <span className="text-muted-foreground">{list.providers.get(provider) ?? 0}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      ) : null}
      {isNarrowed(filter) && (
        <Button variant="ghost" size="sm" onClick={() => change(clearNarrowing(filter))}>
          <XIcon /> Reset
        </Button>
      )}
      <span className="flex-1" />
      {list.nested && (
        <ToggleGroup
          size="sm"
          variant="outline"
          spacing={0}
          aria-label="Depth"
          value={[filter.depth]}
          onValueChange={(values: unknown[]) => {
            if (values[0]) change({ depth: values[0] as ChildDepth });
          }}
        >
          <ToggleGroupItem value="direct" className="data-pressed:bg-accent data-pressed:text-foreground" title="Only the threads this one started">
            Direct
          </ToggleGroupItem>
          <ToggleGroupItem value="all" className="data-pressed:bg-accent data-pressed:text-foreground" title="Their threads too, under them">
            All levels
          </ToggleGroupItem>
        </ToggleGroup>
      )}
      <Choice
        size="sm"
        aria-label="Order"
        value={filter.sort}
        options={(Object.keys(childSortLabel) as ChildSort[]).map((sort) => ({ value: sort, label: childSortLabel[sort] }))}
        onChange={(sort) => sort && change({ sort: sort as ChildSort })}
      />
      {list.archived > 0 && <ArchiveToggle count={list.archived} shown={filter.archived} change={change} />}
    </div>
  );
}

/** Archived agents on and off, as the sidebar's foot does it for threads. */
function ArchiveToggle({ count, shown, change }: { count: number; shown: boolean; change: (change: Partial<ChildFilter>) => void }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Toggle
            size="sm"
            className="gap-1 px-2 text-muted-foreground tabular-nums aria-pressed:bg-card aria-pressed:text-foreground"
            aria-label={`Show archived agents (${count})`}
            pressed={shown}
            onPressedChange={(archived) => change({ archived })}
          />
        }
      >
        <ArchiveIcon />
        {count}
      </TooltipTrigger>
      <TooltipContent side="bottom">{shown ? "Hide" : "Show"} archived agents</TooltipContent>
    </Tooltip>
  );
}

function EmptyList({
  filter,
  list,
  change,
}: {
  filter: ChildFilter;
  list: ChildList;
  change: (change: Partial<ChildFilter>) => void;
}) {
  if (list.total === 0) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <NetworkIcon />
          </EmptyMedia>
          <EmptyTitle>No agents yet</EmptyTitle>
          <EmptyDescription>Threads this agent starts through qcc show up here.</EmptyDescription>
        </EmptyHeader>
      </Empty>
    );
  }
  if (isNarrowed(filter)) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <SearchXIcon />
          </EmptyMedia>
          <EmptyTitle>No agents match the filters</EmptyTitle>
          <EmptyDescription>
            There are {plural(list.total - (filter.archived ? 0 : list.archived), "agent")} here, but none match the search and filters.
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button variant="outline" size="sm" onClick={() => change(clearNarrowing(filter))}>
            <XIcon /> Reset filters
          </Button>
        </EmptyContent>
      </Empty>
    );
  }
  // Nothing narrows the list, so everything in reach is archived.
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ArchiveIcon />
        </EmptyMedia>
        <EmptyTitle>All agents are archived</EmptyTitle>
        <EmptyDescription>The threads this one started have been put away.</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button variant="outline" size="sm" onClick={() => change({ archived: true })}>
          <ArchiveIcon /> Show archived ({list.archived})
        </Button>
      </EmptyContent>
    </Empty>
  );
}

/** How far a row of the tree stands in. */
const indent = (depth: number) => ({ marginLeft: `${depth * 1.5}rem` });

/** A thread shown only because something under it matches: its name, to place the match. */
function PathRow({ row }: { row: ChildRow }) {
  const attention = attentionOf(row.thread);
  return (
    <div className="flex min-w-0 items-center gap-2 px-3 text-sm text-muted-foreground" style={indent(row.depth)}>
      <span className={`dot ${attention}`} title={attentionLabel[attention]} />
      <Button
        variant="link"
        className="h-auto min-w-0 truncate p-0 text-muted-foreground"
        onClick={() => store().select(row.thread.id)}
      >
        {threadTitle(row.thread)}
      </Button>
      {row.thread.archived && <Badge variant="outline">Archived</Badge>}
    </div>
  );
}

function ChildCard({ row, nestedShown }: { row: ChildRow; nestedShown: boolean }) {
  const { thread: child, depth } = row;
  const plugins = useApp((state) => state.allPlugins);
  const now = useNow(1000);
  const attention = attentionOf(child);
  const provider = providerName(plugins, child.provider);
  return (
    <article className={cn("child-card", attention, child.archived && "opacity-70")} style={indent(depth)}>
      <header>
        <span className={`dot ${attention}`} title={attentionLabel[attention]} />
        <Button
          variant="link"
          className="h-auto min-w-0 justify-start truncate p-0 font-semibold text-foreground"
          onClick={() => store().select(child.id)}
        >
          {threadTitle(child)}
        </Button>
        {child.archived && <Badge variant="outline">Archived</Badge>}
        {/* Out of sight in the direct view, a child's own agents are at least counted. */}
        {!nestedShown && row.children > 0 && (
          <Badge variant="secondary" title="Threads this one started">
            <NetworkIcon /> {row.children}
          </Badge>
        )}
        <span className="flex-1" />
        <span className="child-facts">
          {provider}
          {child.environment ? " · worktree" : ""}
        </span>
        <span className="child-time">{cardTime(child, now)}</span>
      </header>
      <div className={`child-status${attention === "active" && child.activity?.kind === "command" ? " technical" : ""}`}>
        {childStatus(child)}
      </div>
      {(child.pending_interactions ?? []).map((pending) => (
        <InteractionPrompt key={pending.id} threadId={child.id} id={pending.id} request={pending.request} />
      ))}
    </article>
  );
}
