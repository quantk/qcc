// The children of a thread at a glance, beside the composer: a chip that counts them
// by what they are up to, and a short list behind it that leads to each of them.

import { ChevronRightIcon, NetworkIcon } from "lucide-react";
import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import { tallyChildren, tallyLabel, tallyParts } from "../../state/lineage";
import { attentionLabel, attentionOf, cardTime, threadTitle } from "../../state/selectors";
import { store } from "../../state/store";
import { showThreadBlock, useBlockSlot } from "../../ui/blocks";
import { useNow } from "../../util/useNow";
import { Button } from "@/components/ui/button";
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from "@/components/ui/item";
import { cn } from "@/lib/utils";
import { AGENTS, childStatus } from "./LineageViews";

/** The face of the chip: how many children, and how many wait, work, failed or are done. */
export function ChildrenChip({ children }: { children: readonly Thread[] }) {
  const tally = useMemo(() => tallyChildren(children), [children]);
  return (
    <span className="flex items-center gap-2 tabular-nums" title={tallyLabel(tally)} aria-label={tallyLabel(tally)}>
      <span className="flex items-center gap-1 text-foreground">
        <NetworkIcon />
        {tally.total}
      </span>
      {tallyParts(tally).map((part) => (
        <span
          key={part.key}
          className={cn("flex items-center gap-1", part.key === "waiting" && "font-medium text-(--blue)")}
          data-attention={part.key === "waiting" ? "" : undefined}
        >
          <span className={`dot ${part.key === "working" ? "active" : part.key === "done" ? "idle" : part.key}`} />
          {part.count}
        </span>
      ))}
    </span>
  );
}

/** The children in short: each one's state and a way to it, the ones that want the user first. */
export function ChildrenGlance({ children }: { children: readonly Thread[] }) {
  const now = useNow(1000);
  const tally = useMemo(() => tallyChildren(children), [children]);
  const agentsSlot = useBlockSlot(AGENTS);
  const canOpenAgents = agentsSlot === "thread.tabs" || agentsSlot === "thread.panel";
  const ordered = useMemo(
    () => [...children].sort((a, b) => urgency[attentionOf(a)] - urgency[attentionOf(b)]),
    [children],
  );
  return (
    <div className="flex flex-col gap-1">
      <p className="px-1 text-xs text-muted-foreground">{tallyLabel(tally)}</p>
      <ul className="flex flex-col">
        {ordered.map((child) => {
          const attention = attentionOf(child);
          return (
            <li key={child.id}>
              <Item
                size="xs"
                className="flex-nowrap px-1.5 py-1.5 hover:bg-muted"
                render={<button type="button" onClick={() => store().select(child.id)} title={`Open ${threadTitle(child)}`} />}
              >
                <ItemMedia>
                  <span className={`dot ${attention}`} title={attentionLabel[attention]} />
                </ItemMedia>
                <ItemContent className="min-w-0">
                  <ItemTitle className="w-full truncate font-normal">{threadTitle(child)}</ItemTitle>
                  <ItemDescription
                    className={cn(
                      "line-clamp-1",
                      attention === "waiting" && "text-(--blue)",
                      attention === "failed" && "text-destructive",
                    )}
                  >
                    {childStatus(child)}
                  </ItemDescription>
                </ItemContent>
                <ItemActions className="text-xs text-muted-foreground tabular-nums">
                  {cardTime(child, now)}
                  <ChevronRightIcon className="size-3.5" />
                </ItemActions>
              </Item>
            </li>
          );
        })}
      </ul>
      {canOpenAgents && (
        <Button variant="ghost" size="xs" className="self-start text-muted-foreground" onClick={() => showThreadBlock(AGENTS)}>
          Answer and follow them in Agents
        </Button>
      )}
    </div>
  );
}

const urgency = { waiting: 0, failed: 1, active: 2, idle: 3, archived: 4 } as const;
