import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useState } from "react";

import { BlockContent, BlockMenu, useSlot } from "../ui/blocks";
import type { Block } from "../ui/registry";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/**
 * The sidebar's own slot: blocks the user wants always in view, stacked under the
 * thread list. Each folds away under its title and keeps to a part of the height,
 * so the thread list stays usable whatever is put here.
 */
export function SidebarBlocks() {
  const blocks = useSlot("sidebar").filter((block) => block.scope === "app");
  if (blocks.length === 0) return null;
  return (
    <div className="flex max-h-[55%] min-h-0 flex-none flex-col overflow-y-auto border-t">
      {blocks.map((block) => (
        <SidebarBlock key={block.id} block={block} />
      ))}
    </div>
  );
}

function SidebarBlock({ block }: { block: Block }) {
  const [open, setOpen] = useState(true);
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="flex min-h-0 flex-col border-b last:border-b-0">
      <BlockMenu
        block={block}
        render={
          <CollapsibleTrigger className="flex w-full items-center gap-1 px-3 py-1.5 text-left text-xs font-semibold tracking-wide text-muted-foreground uppercase hover:text-foreground" />
        }
      >
        <Chevron className="size-3.5" />
        {block.title}
      </BlockMenu>
      <CollapsibleContent className="flex max-h-80 min-h-0 flex-col overflow-y-auto">
        <BlockContent block={block} />
      </CollapsibleContent>
    </Collapsible>
  );
}
