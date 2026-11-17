// Where the window's blocks stand. A block is a piece of UI with an id, a title and
// the context it needs; a slot is a place in the window that shows blocks. Each block
// says where it goes by default, and the user's layout moves, orders or hides blocks
// over that. Only what the user changed is stored, so a block that arrives later (a
// plugin's, on the next connection) takes its own default place, and a block that is
// gone for now keeps its place for when it comes back.

/** What a block needs to be drawn: nothing, or a thread. */
export type Scope = "app" | "thread";

export type SlotId = "views" | "views.foot" | "sidebar" | "thread.tabs" | "thread.panel";

export type SlotInfo = {
  id: SlotId;
  title: string;
  hint: string;
  /** The context the slot gives its blocks. A thread slot gives the app's too. */
  provides: Scope;
};

/** The slots of the window, in the order the settings list them. */
export const SLOTS: readonly SlotInfo[] = [
  { id: "views", title: "Screens", hint: "Buttons at the top of the sidebar, each a screen of its own", provides: "app" },
  { id: "views.foot", title: "Setup screens", hint: "Buttons at the foot of the sidebar, out of the threads' way", provides: "app" },
  { id: "sidebar", title: "Sidebar", hint: "Sections at the bottom of the sidebar, always in view", provides: "app" },
  { id: "thread.tabs", title: "Thread tabs", hint: "Tabs of the thread view", provides: "thread" },
  { id: "thread.panel", title: "Thread side panel", hint: "Tabs of the panel beside a thread", provides: "thread" },
];

export function slotInfo(slot: SlotId): SlotInfo {
  return SLOTS.find((info) => info.id === slot) as SlotInfo;
}

/** Where a block stands: a slot, or none (hidden), and its order within the slot. */
export type Placement = { slot: SlotId | null; order: number };

/** What the layout needs to know of a block. */
export type BlockInfo = { id: string; scope: Scope; placement: Placement };

/** The user's changes to the default places, by block id. */
export type Layout = Readonly<Record<string, Placement>>;

export const defaultLayout: Layout = {};

/** Whether a slot can show a block: a block that needs a thread goes only where there is one. */
export function fits(scope: Scope, slot: SlotId): boolean {
  return scope === "app" || slotInfo(slot).provides === "thread";
}

/** Where a block stands under a layout. A place that no longer fits it is ignored. */
export function placementOf(block: BlockInfo, layout: Layout): Placement {
  const chosen = layout[block.id];
  if (chosen && (chosen.slot === null || fits(block.scope, chosen.slot))) return chosen;
  return block.placement;
}

export type Arrangement<B> = { slots: Record<SlotId, B[]>; hidden: B[] };

const byOrder = (a: { order: number; id: string }, b: { order: number; id: string }) =>
  a.order - b.order || a.id.localeCompare(b.id);

/** The blocks of each slot in order, and those the layout leaves out. */
export function arrange<B extends BlockInfo>(blocks: readonly B[], layout: Layout): Arrangement<B> {
  const placed = blocks.map((block) => ({ block, id: block.id, ...placementOf(block, layout) }));
  placed.sort(byOrder);
  const slots = Object.fromEntries(SLOTS.map((slot) => [slot.id, [] as B[]])) as Record<SlotId, B[]>;
  const hidden: B[] = [];
  for (const { block, slot } of placed) (slot ? slots[slot] : hidden).push(block);
  hidden.sort((a, b) => a.id.localeCompare(b.id));
  return { slots, hidden };
}

/** The blocks one slot shows, in order. */
export function blocksIn<B extends BlockInfo>(blocks: readonly B[], layout: Layout, slot: SlotId): B[] {
  return arrange(blocks, layout).slots[slot];
}

/** Gap between the orders a move writes, so a block arriving later can stand between. */
const STEP = 10;

/**
 * Put a block into a slot at a position (counted among the blocks shown there now,
 * without itself), or hide it with `slot` null. Every block of the target slot gets
 * its order written down, so the order the user sees is the order that stays.
 * A block that cannot go to the slot leaves the layout as it is.
 */
export function moveBlock(
  blocks: readonly BlockInfo[],
  layout: Layout,
  id: string,
  slot: SlotId | null,
  index = Number.POSITIVE_INFINITY,
): Layout {
  const block = blocks.find((candidate) => candidate.id === id);
  if (!block || (slot !== null && !fits(block.scope, slot))) return layout;
  const next: Record<string, Placement> = { ...layout };
  if (slot === null) {
    next[id] = { slot: null, order: 0 };
    return prune(blocks, next);
  }
  const others = blocksIn(blocks, layout, slot).filter((other) => other.id !== id);
  const at = Math.max(0, Math.min(index, others.length));
  const ordered = [...others.slice(0, at), block, ...others.slice(at)];
  ordered.forEach((member, position) => {
    next[member.id] = { slot, order: position * STEP };
  });
  return prune(blocks, next);
}

/** Take back what the user changed about one block. */
export function resetBlock(layout: Layout, id: string): Layout {
  if (!(id in layout)) return layout;
  const next = { ...layout };
  delete next[id];
  return next;
}

/** Drop entries that say what the default says anyway. Unknown blocks are kept. */
function prune(blocks: readonly BlockInfo[], layout: Record<string, Placement>): Layout {
  for (const block of blocks) {
    const chosen = layout[block.id];
    if (chosen && chosen.slot === block.placement.slot && chosen.order === block.placement.order) {
      delete layout[block.id];
    }
  }
  return layout;
}

const SLOT_IDS = new Set<string>(SLOTS.map((slot) => slot.id));

/** A layout read back from storage: anything malformed is left out rather than trusted. */
export function parseLayout(value: unknown): Layout {
  if (!value || typeof value !== "object" || Array.isArray(value)) return defaultLayout;
  const layout: Record<string, Placement> = {};
  for (const [id, entry] of Object.entries(value as Record<string, unknown>)) {
    if (!entry || typeof entry !== "object") continue;
    const { slot, order } = entry as Record<string, unknown>;
    if (typeof order !== "number" || !Number.isFinite(order)) continue;
    if (slot === null || (typeof slot === "string" && SLOT_IDS.has(slot))) {
      layout[id] = { slot: slot as SlotId | null, order };
    }
  }
  return layout;
}

/**
 * Where among a slot's blocks a dragged one lands: before the first block whose
 * middle is below the pointer. `middles` are those of the blocks shown there now,
 * without the dragged one, top to bottom.
 */
export function dropIndex(middles: readonly number[], y: number): number {
  const index = middles.findIndex((middle) => y < middle);
  return index === -1 ? middles.length : index;
}
