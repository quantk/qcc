import { EllipsisIcon } from "lucide-react";

import type { Item, Thread } from "../../api/protocol.gen";
import { useApp } from "../../state/store";
import { cn } from "@/lib/utils";
import { type ItemContext, useUi } from "../registry";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/**
 * The "…" menu under an item of the timeline. Its entries are contributed
 * (`ui.itemMenuItem`); nothing is drawn under an item no entry is offered for. An
 * entry that cannot run now is shown disabled, saying why.
 */
export function ItemMenu(props: { item: Item; thread: Thread; className?: string }) {
  const { item, thread } = props;
  const all = useUi((snapshot) => snapshot.itemMenu);
  const timeline = useApp((state) => state.timelines[thread.id]);
  const context: ItemContext = { item, thread, timeline };
  const entries = all.filter((entry) => entry.when?.(context) ?? true);
  if (entries.length === 0) return null;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label="More about this reply"
            className={cn("text-muted-foreground hover:text-foreground", props.className)}
          />
        }
      >
        <EllipsisIcon />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-auto min-w-52">
        {entries.map((entry) => {
          const blocker = entry.blocker?.(context) ?? null;
          const row = (
            <DropdownMenuItem key={entry.id} disabled={blocker !== null} onClick={() => entry.run(context)}>
              {entry.title}
            </DropdownMenuItem>
          );
          if (blocker === null) return row;
          // A disabled entry takes no pointer, so the reason hangs on what holds it.
          return (
            <Tooltip key={entry.id}>
              <TooltipTrigger render={<div />}>{row}</TooltipTrigger>
              <TooltipContent side="right" className="max-w-72">
                {blocker}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
