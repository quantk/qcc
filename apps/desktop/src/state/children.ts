// The threads a thread started, and their threads in turn: what state each is in,
// how many there are, and which of them a list shows under the user's filters.
// Kept apart from the views so a count elsewhere (a badge, an indicator by the
// composer) reads the same numbers the Agents tab does.

import { useSyncExternalStore } from "react";

import type { Thread } from "../api/protocol.gen";
import { readJson, saveJson } from "../util/storage";

/**
 * What a delegated thread is up to, finer than `attentionOf`: an idle thread whose
 * last turn completed is done, one that never ran or was stopped is only idle.
 * Archiving is not a state but a filter of its own, so an archived thread still has one.
 */
export type ChildState = "waiting" | "working" | "failed" | "done" | "idle";

/** The states in the order a filter offers them and a status sort puts them. */
export const CHILD_STATES: readonly ChildState[] = ["waiting", "failed", "working", "idle", "done"];

export const childStateLabel: Record<ChildState, string> = {
  waiting: "Waiting",
  working: "Working",
  idle: "Idle",
  done: "Done",
  failed: "Failed",
};

export function childState(thread: Thread): ChildState {
  if ((thread.pending_interactions ?? []).length > 0) return "waiting";
  if (thread.status === "active") return "working";
  if (thread.status === "failed" || thread.last_turn_status === "failed") return "failed";
  if (thread.last_turn_status === "completed") return "done";
  return "idle";
}

/** A guard against a cycle of parents, not a limit anyone reaches. */
const MAX_DEPTH = 32;

/** A descendant and how far below the thread it is: 1 for a child, 2 for a grandchild. */
export type Descendant = { thread: Thread; depth: number };

/**
 * The threads under `id`, each parent before its children and siblings in the order
 * they were started. A plugin's own hidden threads are left out, as every list does.
 */
export function descendantsOf(threads: readonly Thread[], id: string, maxDepth = MAX_DEPTH): Descendant[] {
  const byParent = childIndex(threads);
  const out: Descendant[] = [];
  const visit = (parent: string, depth: number) => {
    if (depth > Math.min(maxDepth, MAX_DEPTH)) return;
    for (const thread of byParent.get(parent) ?? []) {
      out.push({ thread, depth });
      visit(thread.id, depth + 1);
    }
  };
  visit(id, 1);
  return out;
}

function childIndex(threads: readonly Thread[]): Map<string, Thread[]> {
  const byParent = new Map<string, Thread[]>();
  for (const thread of threads) {
    if (!thread.parent || thread.hidden) continue;
    const siblings = byParent.get(thread.parent) ?? [];
    siblings.push(thread);
    byParent.set(thread.parent, siblings);
  }
  for (const siblings of byParent.values()) siblings.sort((a, b) => a.created_at - b.created_at);
  return byParent;
}

/** How many threads stand under a thread, by state: what a badge or an indicator shows. */
export type ChildCounts = {
  /** Everything under it, archived included. */
  total: number;
  archived: number;
  /** Of the threads not archived. */
  byState: Record<ChildState, number>;
};

export function countChildren(threads: readonly Thread[], id: string, maxDepth = MAX_DEPTH): ChildCounts {
  const counts: ChildCounts = { total: 0, archived: 0, byState: emptyStates() };
  for (const { thread } of descendantsOf(threads, id, maxDepth)) {
    counts.total++;
    if (thread.archived) counts.archived++;
    else counts.byState[childState(thread)]++;
  }
  return counts;
}

function emptyStates(): Record<ChildState, number> {
  return { waiting: 0, working: 0, failed: 0, done: 0, idle: 0 };
}

export type ChildSort = "started" | "recent" | "status";

export const childSortLabel: Record<ChildSort, string> = {
  started: "Order started",
  recent: "Recently active",
  status: "Status",
};

/** How deep the list reaches: the threads this one started, or theirs too. */
export type ChildDepth = "direct" | "all";

/** What the Agents tab shows. Empty `states` or `providers` means any. */
export type ChildFilter = {
  states: readonly ChildState[];
  providers: readonly string[];
  /** Matched against the title, case aside. */
  query: string;
  archived: boolean;
  depth: ChildDepth;
  sort: ChildSort;
};

export const DEFAULT_CHILD_FILTER: ChildFilter = {
  states: [],
  providers: [],
  query: "",
  archived: false,
  depth: "all",
  sort: "started",
};

/** Whether anything narrows the list beyond the archive and the depth, which are how it is viewed. */
export function isNarrowed(filter: ChildFilter): boolean {
  return filter.states.length > 0 || filter.providers.length > 0 || filter.query.trim() !== "";
}

/** The filter with nothing narrowing it; how the list is viewed stays. */
export function clearNarrowing(filter: ChildFilter): ChildFilter {
  return { ...filter, states: [], providers: [], query: "" };
}

export type ChildRow = {
  thread: Thread;
  /** 0 for a child, 1 for a grandchild: the indent. */
  depth: number;
  /**
   * False for a thread shown only because something under it matches: the tree
   * keeps the path to a match, and the view dims what is there only for the path.
   */
  match: boolean;
  /** Threads it started, whether shown or not. */
  children: number;
};

export type ChildList = {
  rows: ChildRow[];
  /** Threads in reach of the depth, archived included. */
  total: number;
  /** Of those, archived: the count on the archive toggle. */
  archived: number;
  /** Rows that match, not those kept for their path. */
  shown: number;
  /** Whether any thread in reach started threads of its own: only then is the depth a choice. */
  nested: boolean;
  /**
   * How many threads each state and provider would show, with the archive and the
   * depth applied but not the other filters: the counts on the filter's options.
   */
  states: Record<ChildState, number>;
  providers: Map<string, number>;
};

const URGENCY = Object.fromEntries(CHILD_STATES.map((state, index) => [state, index])) as Record<ChildState, number>;

const COMPARE: Record<ChildSort, (a: Thread, b: Thread) => number> = {
  started: (a, b) => a.created_at - b.created_at,
  recent: (a, b) => b.updated_at - a.updated_at,
  status: (a, b) =>
    URGENCY[childState(a)] - URGENCY[childState(b)] || Number(!!a.archived) - Number(!!b.archived) || b.updated_at - a.updated_at,
};

/**
 * The threads under `id` the Agents tab lists. With `depth: "all"` the list is a
 * tree: siblings sorted, each thread followed by its own, and a thread that does
 * not match kept when something under it does, so a match never loses its place.
 */
export function childList(threads: readonly Thread[], id: string, filter: ChildFilter): ChildList {
  const byParent = childIndex(threads);
  const reach = filter.depth === "direct" ? 1 : MAX_DEPTH;
  const query = filter.query.trim().toLocaleLowerCase();
  const states = new Set(filter.states);
  const providers = new Set(filter.providers);

  const list: ChildList = {
    rows: [],
    total: 0,
    archived: 0,
    shown: 0,
    nested: false,
    states: emptyStates(),
    providers: new Map(),
  };
  const matches = (thread: Thread): boolean => {
    if (thread.archived && !filter.archived) return false;
    if (states.size > 0 && !states.has(childState(thread))) return false;
    if (providers.size > 0 && !providers.has(thread.provider)) return false;
    return !query || (thread.title ?? "").toLocaleLowerCase().includes(query);
  };

  // Returns the rows of the subtree under `parent`, for the caller to keep or drop.
  const visit = (parent: string, depth: number): ChildRow[] => {
    if (depth >= reach) return [];
    const siblings = [...(byParent.get(parent) ?? [])].sort((a, b) => COMPARE[filter.sort](a, b) || a.created_at - b.created_at);
    const rows: ChildRow[] = [];
    for (const thread of siblings) {
      const own = byParent.get(thread.id)?.length ?? 0;
      list.total++;
      if (own > 0) list.nested = true;
      if (thread.archived) list.archived++;
      if (!thread.archived || filter.archived) {
        list.states[childState(thread)]++;
        list.providers.set(thread.provider, (list.providers.get(thread.provider) ?? 0) + 1);
      }
      const match = matches(thread);
      const below = visit(thread.id, depth + 1);
      if (match) list.shown++;
      if (match || below.length > 0) rows.push({ thread, depth, match, children: own }, ...below);
    }
    return rows;
  };
  list.rows = visit(id, 0);
  return list;
}

// How the list is viewed (the archive, the depth, the order) is a preference: it is
// kept across threads and restarts. What narrows it (states, providers, a search) is
// about the threads in front of the user right now: it is kept per parent thread
// while the window lives, so switching tabs or threads does not lose it, but a
// narrowing forgotten last week never hides agents from a new thread.

const VIEW_KEY = "qcc.agents-view";

type ViewPrefs = Pick<ChildFilter, "archived" | "depth" | "sort">;

export function parseViewPrefs(saved: unknown): ViewPrefs {
  const prefs: ViewPrefs = { archived: false, depth: "all", sort: "started" };
  if (!saved || typeof saved !== "object") return prefs;
  const value = saved as Record<string, unknown>;
  if (typeof value.archived === "boolean") prefs.archived = value.archived;
  if (value.depth === "direct" || value.depth === "all") prefs.depth = value.depth;
  if (value.sort === "started" || value.sort === "recent" || value.sort === "status") prefs.sort = value.sort;
  return prefs;
}

type Narrowing = Pick<ChildFilter, "states" | "providers" | "query">;

/** The filters of every parent thread's Agents tab. */
export class ChildFilters {
  private prefs: ViewPrefs;
  private narrowing = new Map<string, Narrowing>();
  private cache = new Map<string, ChildFilter>();
  private listeners = new Set<() => void>();

  constructor(private readonly persist: { load(): unknown; save(prefs: ViewPrefs): void }) {
    this.prefs = parseViewPrefs(persist.load());
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** The same object until something changes, as `useSyncExternalStore` needs. */
  get(parent: string): ChildFilter {
    let filter = this.cache.get(parent);
    if (!filter) {
      const narrowing = this.narrowing.get(parent) ?? { states: [], providers: [], query: "" };
      filter = { ...this.prefs, ...narrowing };
      this.cache.set(parent, filter);
    }
    return filter;
  }

  set(parent: string, change: Partial<ChildFilter>): void {
    const next = { ...this.get(parent), ...change };
    const prefs: ViewPrefs = { archived: next.archived, depth: next.depth, sort: next.sort };
    if (prefs.archived !== this.prefs.archived || prefs.depth !== this.prefs.depth || prefs.sort !== this.prefs.sort) {
      this.prefs = prefs;
      this.persist.save(prefs);
      // Every parent sees the new preference.
      this.cache.clear();
    }
    this.narrowing.set(parent, { states: next.states, providers: next.providers, query: next.query });
    this.cache.delete(parent);
    for (const listener of this.listeners) listener();
  }
}

export const childFilters = new ChildFilters({
  load: () => readJson(VIEW_KEY),
  save: (prefs) => saveJson(VIEW_KEY, prefs),
});

export function useChildFilter(parent: string): [ChildFilter, (change: Partial<ChildFilter>) => void] {
  const filter = useSyncExternalStore(childFilters.subscribe, () => childFilters.get(parent));
  return [filter, (change) => childFilters.set(parent, change)];
}
