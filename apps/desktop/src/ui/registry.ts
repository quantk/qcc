// The extension points of the window. Everything on screen beyond the bare shell is
// a contribution: the timeline and the changes tab register here exactly as a
// plugin's panel does, so nothing built in has a private way in. Blocks say what
// they are and where they go by default; which slot shows them is the layout's call.

import { type ComponentType, useSyncExternalStore } from "react";

import type { Item, ProviderInfo, Thread } from "../api/protocol.gen";
import type { Placement } from "../state/blocks";
import type { Timing } from "../state/durations";
import type { ThreadGroup } from "../state/selectors";
import type { Timeline } from "../state/timeline";

/** What a contribution about one thread is given. */
export type ThreadContext = {
  thread: Thread;
  /** Absent until the thread's events are read. */
  timeline: Timeline | undefined;
  provider: ProviderInfo | undefined;
  /** The thread that started this one, if it is known. */
  parent: Thread | undefined;
  /** The threads this one started, oldest first. */
  children: readonly Thread[];
};

/**
 * A unit of the window's UI that can stand in any slot whose context it fits: a
 * screen, a section of the sidebar, a tab of a thread or of its side panel. Where it
 * stands by default is `placement`; the user's layout (`state/blocks.ts`) may move,
 * reorder or hide it.
 */
type BlockBase = {
  /** Unique among all blocks; the layout remembers places by it. */
  id: string;
  title: string;
  /** Where it goes until the user moves it. A null slot: offered, but not shown. */
  placement: Placement;
  /** A line that tells it apart in the layout settings: what it shows, whose it is. */
  hint?: string;
  /**
   * Drawn where a block is a button rather than a tab: a screen in the sidebar shows
   * only this, with the title as its tooltip. Absent: a generic icon.
   */
  icon?: ComponentType<{ className?: string }>;
  /**
   * A number to show on the block's button, such as how many agents a screen has to
   * show. Called as a React hook while the button is drawn, so it may read the store.
   */
  useBadge?: () => ViewBadge | null;
};

/** A block that needs nothing: it can stand anywhere. */
export type AppBlock = BlockBase & {
  scope: "app";
  render: ComponentType;
  /** Its face where it stands as a chip (beside the composer); see `ThreadBlock.summary`. */
  summary?: ComponentType;
};

/** A block about one thread: it stands only in a thread's slots. */
export type ThreadBlock = BlockBase & {
  scope: "thread";
  /** Absent: shown for every thread. */
  when?: (context: ThreadContext) => boolean;
  render: ComponentType<ThreadContext>;
  /**
   * Its face where it stands as a chip, beside the composer: a few words or counts
   * that say enough at a glance, drawn inside the chip's button; a click shows
   * `render` in a popover. An element marked `data-attention` tints the chip, for
   * something that waits for the user. Absent: the icon and the title.
   */
  summary?: ComponentType<ThreadContext>;
};

export type Block = AppBlock | ThreadBlock;

/** Whether a block has something to show for a thread; one that needs nothing always does. */
export function blockApplies(block: Block, context: ThreadContext): boolean {
  return block.scope === "app" || (block.when?.(context) ?? true);
}

/** A count on a view's button. */
export type ViewBadge = {
  count: number;
  /** `attention`: something there waits for the user. */
  tone?: "default" | "attention";
  /** What the count means, added to the tooltip. */
  label: string;
};

/** A control in the thread's header. */
export type ThreadAction = {
  id: string;
  order: number;
  when?: (context: ThreadContext) => boolean;
  render: ComponentType<ThreadContext>;
};

export type ItemRendererProps = {
  item: Item;
  thread: Thread;
  /** When the item started and, once settled, ended. Absent where the timeline does not know. */
  timing?: Timing;
  /** For the last reply of an ended turn: when the whole turn started and ended. */
  turn?: Timing;
};

/** What an entry of an item's menu is given: the item, and the thread it is of. */
export type ItemContext = { item: Item; thread: Thread; timeline: Timeline | undefined };

/**
 * An entry of the "…" menu under an item of the timeline, beside its copy button.
 * The menu shows only under items some entry is offered for.
 */
export type ItemMenuItem = {
  id: string;
  title: string;
  order: number;
  /** Absent: offered under every item. */
  when?: (context: ItemContext) => boolean;
  /** Why it cannot be run now, which the entry says; null: it can. Absent: always can. */
  blocker?: (context: ItemContext) => string | null;
  run: (context: ItemContext) => void;
};

/** Draws one kind of timeline item. */
export type ItemRenderer = ComponentType<ItemRendererProps>;

/** Something the user can run from the palette or by a shortcut. */
export type Command = {
  id: string;
  title: string;
  /**
   * `Ctrl+N`, `Ctrl+Shift+P`: modifiers then one key. Ctrl also matches Cmd. `Plus`
   * and `Minus` name those keys, with or without Shift.
   */
  shortcut?: string;
  /** Absent: always available. */
  enabled?: () => boolean;
  run: () => void;
};

/**
 * An entry of a thread's context menu (right click on the thread in the sidebar
 * or the overview). Entries with the same `group` stand together, groups apart.
 */
export type ThreadMenuItem = {
  id: string;
  title: string | ((thread: Thread) => string);
  order: number;
  group?: string;
  destructive?: boolean;
  /** Absent: offered for every thread. */
  when?: (thread: Thread) => boolean;
  run: (thread: Thread) => void;
};

/** A button on the header of a sidebar group: a project, or a folder of threads in no project. */
export type GroupAction = {
  id: string;
  order: number;
  /** Absent: offered for every group. */
  when?: (group: ThreadGroup) => boolean;
  /** What the button does, for screen readers. */
  label: (group: ThreadGroup) => string;
  /** The tooltip. */
  title: string;
  icon: ComponentType;
  /** Shown only while the pointer is over the group, for actions rarely wanted. */
  onHover?: boolean;
  run: (group: ThreadGroup) => void;
};

/** A modal the shell shows on a contribution's behalf. */
export type DialogRenderer = ComponentType<{ close: () => void }>;

/** A unit of UI: built in or made from a plugin's contributions. */
export type UiModule = {
  id: string;
  /** Register contributions. Whatever it returns undoes them. */
  activate(ui: UiRegistry): void | (() => void);
};

/** The key an item's renderer is registered under. */
export function itemKey(item: Item): string {
  return item.type === "extension" ? `extension:${item.kind}` : item.type;
}

type Snapshot = {
  blocks: readonly Block[];
  actions: readonly ThreadAction[];
  commands: readonly Command[];
  menu: readonly ThreadMenuItem[];
  itemMenu: readonly ItemMenuItem[];
  groupActions: readonly GroupAction[];
  items: ReadonlyMap<string, ItemRenderer>;
  dialog: DialogRenderer | null;
};

const byOrder = <T extends { order: number; id: string }>(a: T, b: T) =>
  a.order - b.order || a.id.localeCompare(b.id);

export class UiRegistry {
  private snapshot: Snapshot = {
    blocks: [],
    actions: [],
    commands: [],
    menu: [],
    itemMenu: [],
    groupActions: [],
    items: new Map(),
    dialog: null,
  };
  private listeners = new Set<() => void>();
  private active = new Map<string, () => void>();
  private collecting: (() => void)[] | null = null;

  getSnapshot = (): Snapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private update(patch: Partial<Snapshot>) {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }

  /** Turn a module on. Activating an id again replaces what it registered before. */
  activate(module: UiModule): void {
    this.deactivate(module.id);
    // Everything registered while the module activates is remembered, so the
    // module can be taken out whole.
    const undo: (() => void)[] = [];
    this.collecting = undo;
    try {
      const cleanup = module.activate(this);
      if (cleanup) undo.push(cleanup);
    } finally {
      this.collecting = null;
    }
    this.active.set(module.id, () => undo.forEach((remove) => remove()));
  }

  private track(remove: () => void): () => void {
    this.collecting?.push(remove);
    return remove;
  }

  deactivate(id: string): void {
    this.active.get(id)?.();
    this.active.delete(id);
  }

  activeModules(): string[] {
    return [...this.active.keys()];
  }

  /** Offer a block. One with the same id is replaced. */
  block(block: Block): () => void {
    this.update({ blocks: [...this.snapshot.blocks.filter((other) => other.id !== block.id), block] });
    return this.track(() => this.update({ blocks: this.snapshot.blocks.filter((other) => other !== block) }));
  }

  threadMenuItem(item: ThreadMenuItem): () => void {
    this.update({ menu: [...this.snapshot.menu.filter((other) => other.id !== item.id), item].sort(byOrder) });
    return this.track(() => this.update({ menu: this.snapshot.menu.filter((other) => other !== item) }));
  }

  itemMenuItem(item: ItemMenuItem): () => void {
    this.update({
      itemMenu: [...this.snapshot.itemMenu.filter((other) => other.id !== item.id), item].sort(byOrder),
    });
    return this.track(() => this.update({ itemMenu: this.snapshot.itemMenu.filter((other) => other !== item) }));
  }

  groupAction(action: GroupAction): () => void {
    this.update({
      groupActions: [...this.snapshot.groupActions.filter((other) => other.id !== action.id), action].sort(byOrder),
    });
    return this.track(() =>
      this.update({ groupActions: this.snapshot.groupActions.filter((other) => other !== action) }),
    );
  }

  threadAction(action: ThreadAction): () => void {
    this.update({
      actions: [...this.snapshot.actions.filter((other) => other.id !== action.id), action].sort(byOrder),
    });
    return this.track(() => this.update({ actions: this.snapshot.actions.filter((other) => other !== action) }));
  }

  command(command: Command): () => void {
    this.update({ commands: [...this.snapshot.commands.filter((other) => other.id !== command.id), command] });
    return this.track(() => this.update({ commands: this.snapshot.commands.filter((other) => other !== command) }));
  }

  /**
   * Draw items of one kind: a built-in item type (`command`, `fileChange`) or a
   * plugin's own (`extension:<plugin-id>/<name>`). The last registration wins.
   */
  itemRenderer(key: string, renderer: ItemRenderer): () => void {
    const previous = this.snapshot.items.get(key);
    this.update({ items: new Map(this.snapshot.items).set(key, renderer) });
    return this.track(() => {
      if (this.snapshot.items.get(key) !== renderer) return;
      const items = new Map(this.snapshot.items);
      if (previous) items.set(key, previous);
      else items.delete(key);
      this.update({ items });
    });
  }

  showDialog(dialog: DialogRenderer): void {
    this.update({ dialog });
  }

  closeDialog = (): void => {
    this.update({ dialog: null });
  };
}

/** The one registry of the window. */
export const ui = new UiRegistry();

export function useUi<T>(select: (snapshot: Snapshot) => T): T {
  return useSyncExternalStore(ui.subscribe, () => select(ui.getSnapshot()));
}

/**
 * Keys named in a shortcut rather than typed: `+` would split it, and both share a
 * physical key with another character, so Shift is not part of them.
 */
const NAMED_KEYS: Record<string, readonly string[]> = {
  plus: ["+", "="],
  minus: ["-", "_"],
  left: ["arrowleft"],
  right: ["arrowright"],
};

/** Whether a keyboard event is the shortcut a command asks for. */
export function matchesShortcut(event: KeyboardEvent, shortcut: string): boolean {
  const parts = shortcut.toLowerCase().split("+");
  const key = parts.pop() ?? "";
  const named = NAMED_KEYS[key];
  const pressed = event.key.toLowerCase();
  return (
    (named ? named.includes(pressed) : pressed === key) &&
    (event.ctrlKey || event.metaKey) === parts.includes("ctrl") &&
    (named !== undefined || event.shiftKey === parts.includes("shift")) &&
    event.altKey === parts.includes("alt")
  );
}
