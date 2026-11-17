// Blocks as the shell sees them: which ones a slot shows under the user's layout,
// how one is drawn with the context it needs, and how the user moves one.

import { type ReactElement, type ReactNode, useMemo } from "react";

import { type SlotId, SLOTS, arrange, blocksIn, fits, moveBlock, placementOf } from "../state/blocks";
import { store, useApp } from "../state/store";
import { type Block, type ThreadContext, ui, useUi } from "./registry";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";

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
