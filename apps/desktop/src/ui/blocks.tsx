// Blocks as the shell sees them: which ones a slot shows under the user's layout,
// how one is drawn with the context it needs, and how the user moves one.

import { LayoutGridIcon } from "lucide-react";
import { type ReactElement, type ReactNode, useMemo } from "react";

import { type SlotId, SLOTS, arrange, blocksIn, fits, moveBlock, placementOf } from "../state/blocks";
import { store, useApp } from "../state/store";
import { type Block, type ThreadContext, blockApplies, ui, useUi } from "./registry";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Popover, PopoverContent, PopoverHeader, PopoverTitle, PopoverTrigger } from "@/components/ui/popover";

/** The blocks a slot shows, in order. A thread's slot still has to ask each `blockApplies`. */
export function useSlot(slot: SlotId): Block[] {
  const blocks = useUi((snapshot) => snapshot.blocks);
  const layout = useApp((state) => state.blockLayout);
  return useMemo(() => blocksIn(blocks, layout, slot), [blocks, layout, slot]);
}

/** Every slot's blocks and the hidden ones. */
export function useArrangement() {
  const blocks = useUi((snapshot) => snapshot.blocks);
  const layout = useApp((state) => state.blockLayout);
  return useMemo(() => arrange(blocks, layout), [blocks, layout]);
}

/** Put a block into a slot at a position among the others there, or hide it (`null`). */
export function placeBlock(id: string, slot: SlotId | null, index?: number): void {
  const { blockLayout } = store().getState();
  store().setBlockLayout(moveBlock(ui.getSnapshot().blocks, blockLayout, id, slot, index));
}

/** A block drawn with what it needs: a thread's block gets the thread, an app's block nothing. */
export function BlockContent({ block, context }: { block: Block; context?: ThreadContext }) {
  if (block.scope === "app") return <block.render />;
  return context ? <block.render {...context} /> : null;
}

/** The command that opens the layout settings, if a module offers it. */
const ARRANGE_COMMAND = "layout.edit";

/**
 * A block's right-click menu around `render`, the element that names it (a tab):
 * move it to another slot that fits it, hide it, or open the whole layout.
 */
export function BlockMenu(props: { block: Block; render: ReactElement; children: ReactNode }) {
  const { block } = props;
  const layout = useApp((state) => state.blockLayout);
  const arrangeCommand = useUi((snapshot) => snapshot.commands.find((command) => command.id === ARRANGE_COMMAND));
  const current = placementOf(block, layout).slot;
  const targets = SLOTS.filter((slot) => slot.id !== current && fits(block.scope, slot.id));
  return (
    <ContextMenu>
      <ContextMenuTrigger render={props.render}>{props.children}</ContextMenuTrigger>
      <ContextMenuContent className="min-w-52">
        {targets.map((slot) => (
          <ContextMenuItem key={slot.id} onClick={() => placeBlock(block.id, slot.id)}>
            Move to {slot.title.toLowerCase()}
          </ContextMenuItem>
        ))}
        <ContextMenuItem onClick={() => placeBlock(block.id, null)}>Hide</ContextMenuItem>
        {arrangeCommand && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem onClick={arrangeCommand.run}>{arrangeCommand.title}…</ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** The slot a block stands in under the user's layout, or null while it is hidden or unknown. */
export function useBlockSlot(id: string): SlotId | null {
  const block = useUi((snapshot) => snapshot.blocks.find((candidate) => candidate.id === id));
  const layout = useApp((state) => state.blockLayout);
  return block ? placementOf(block, layout).slot : null;
}

/**
 * Bring a block of the selected thread into view where the layout put it: its tab
 * of the thread view or of the side panel. Says whether it could: a block that is
 * hidden or stands elsewhere is not opened behind the user's back.
 */
export function showThreadBlock(id: string): boolean {
  const block = ui.getSnapshot().blocks.find((candidate) => candidate.id === id);
  if (!block) return false;
  const slot = placementOf(block, store().getState().blockLayout).slot;
  if (slot === "thread.tabs") store().openThreadTab(id);
  else if (slot === "thread.panel") store().openSidePanel(id);
  else return false;
  return true;
}

/**
 * A slot whose blocks stand as chips in a bar, such as the composer's: each shows its
 * `summary` (or its icon and title) and opens its content in a popover. A block with
 * nothing to show for the thread takes no room.
 */
export function BlockChips({ slot, context }: { slot: SlotId; context: ThreadContext }) {
  const blocks = useSlot(slot).filter((block) => blockApplies(block, context));
  if (blocks.length === 0) return null;
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      {blocks.map((block) => (
        <BlockChip key={block.id} block={block} context={context} />
      ))}
    </span>
  );
}

function BlockChip({ block, context }: { block: Block; context: ThreadContext }) {
  const Icon = block.icon ?? LayoutGridIcon;
  const face = !block.summary ? (
    <>
      <Icon /> {block.title}
    </>
  ) : block.scope === "app" ? (
    <block.summary />
  ) : (
    <block.summary {...context} />
  );
  return (
    <Popover>
      {/* A right click offers to move or hide the chip, like a tab's. A summary that
          marks something with `data-attention` tints the chip: it wants the user. */}
      <BlockMenu block={block} render={<span className="contents" />}>
        <PopoverTrigger
          render={
            <Button
              type="button"
              variant="outline"
              size="xs"
              title={block.hint ?? block.title}
              className="bg-card text-muted-foreground has-data-attention:border-(--blue) has-data-attention:bg-(--blue)/10 has-data-attention:text-foreground"
            />
          }
        >
          {face}
        </PopoverTrigger>
      </BlockMenu>
      <PopoverContent side="top" align="start" className="max-h-[min(480px,60vh)] w-96 max-w-[calc(100vw-2rem)] gap-2 overflow-y-auto">
        <PopoverHeader>
          <PopoverTitle>{block.title}</PopoverTitle>
        </PopoverHeader>
        <div className="flex min-h-0 flex-col">
          <BlockContent block={block} context={context} />
        </div>
      </PopoverContent>
    </Popover>
  );
}
