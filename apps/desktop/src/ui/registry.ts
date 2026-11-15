// The extension points of the window. Everything on screen beyond the bare shell is
// a contribution: the timeline and the changes tab register here exactly as a
// plugin's panel does, so nothing built in has a private way in.

import { type ComponentType, useSyncExternalStore } from "react";

import type { Item, ProviderInfo, Thread } from "../api/protocol.gen";
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

/** A tab of the thread view. */
export type ThreadTab = {
  id: string;
  title: string;
  /** Tabs are shown in ascending order; the first visible one opens by default. */
  order: number;
  /** Absent: shown for every thread. */
  when?: (context: ThreadContext) => boolean;
  render: ComponentType<ThreadContext>;
};

/** A screen of its own, reachable from the top of the sidebar. */
export type AppView = {
  id: string;
  title: string;
  /** Ascending; the first one is what the app opens with. */
  order: number;
  render: ComponentType;
};

/** A control in the thread's header. */
export type ThreadAction = {
  id: string;
  order: number;
  when?: (context: ThreadContext) => boolean;
  render: ComponentType<ThreadContext>;
};

export type ItemRendererProps = { item: Item; thread: Thread };

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
  views: readonly AppView[];
  tabs: readonly ThreadTab[];
  actions: readonly ThreadAction[];
  commands: readonly Command[];
  items: ReadonlyMap<string, ItemRenderer>;
  dialog: DialogRenderer | null;
};

const byOrder = <T extends { order: number; id: string }>(a: T, b: T) =>
  a.order - b.order || a.id.localeCompare(b.id);

export class UiRegistry {
  private snapshot: Snapshot = { views: [], tabs: [], actions: [], commands: [], items: new Map(), dialog: null };
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

  appView(view: AppView): () => void {
    this.update({ views: [...this.snapshot.views.filter((other) => other.id !== view.id), view].sort(byOrder) });
    return this.track(() => this.update({ views: this.snapshot.views.filter((other) => other !== view) }));
  }

  threadTab(tab: ThreadTab): () => void {
    this.update({ tabs: [...this.snapshot.tabs.filter((other) => other.id !== tab.id), tab].sort(byOrder) });
    return this.track(() => this.update({ tabs: this.snapshot.tabs.filter((other) => other !== tab) }));
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
