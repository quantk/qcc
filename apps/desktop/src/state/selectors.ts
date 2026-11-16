import { elapsed, relativePath, relativeTime, tailPath } from "../util/format";
import type { PermissionMode, PluginInfo, Project, ProviderInfo, Settings, Thread } from "../api/protocol.gen";

/** Example plugins are left out unless asked for. */
export function visiblePlugins(plugins: readonly PluginInfo[], showExamples: boolean): readonly PluginInfo[] {
  return showExamples ? plugins : plugins.filter((plugin) => !plugin.example);
}

export function providersOf(plugins: readonly PluginInfo[]): ProviderInfo[] {
  return plugins.flatMap((plugin) => plugin.providers);
}

/** The agents offered to pick from: the user can hide some in the settings. */
export function pickableProviders(plugins: readonly PluginInfo[], settings: Settings | undefined): ProviderInfo[] {
  return providersOf(plugins).filter((provider) => !settings?.providers?.[provider.id]?.hidden);
}

export function environmentsOf(plugins: readonly PluginInfo[]) {
  return plugins.flatMap((plugin) => plugin.environments);
}

/**
 * Whether the changes of a thread can be shown: its environment is ready, or it
 * works in a directory of its own and some environment can inspect directories.
 */
export function changesShown(thread: Thread, plugins: readonly PluginInfo[]): boolean {
  if (thread.environment) return thread.environment.state === "ready";
  return environmentsOf(plugins).some((environment) => environment.inspects_directories);
}

export function findProvider(plugins: readonly PluginInfo[], id: string): ProviderInfo | undefined {
  return providersOf(plugins).find((provider) => provider.id === id);
}

/** The name an agent goes by: its id when no loaded plugin provides it. */
export function providerName(plugins: readonly PluginInfo[], id: string): string {
  return findProvider(plugins, id)?.name ?? id;
}

/** What a thread is called, before its agent names it too. */
export function threadTitle(thread: Pick<Thread, "title"> | undefined): string {
  return thread?.title ?? "New thread";
}

/** The directory a thread is about: where it was asked to work. */
export function folderOf(thread: Thread): string {
  return thread.environment?.source_dir ?? thread.cwd;
}

/** A group of the sidebar: a project, or a folder of threads in no project. */
export type ThreadGroup =
  | { kind: "project"; key: string; project: Project; threads: Thread[]; latest: number }
  | { kind: "folder"; key: string; folder: string; threads: Thread[]; latest: number };

/**
 * Threads by project, and the rest by folder. A family stays together under the
 * group of the thread the user began, wherever its members work. Projects come
 * first, every one of them even without threads, most recently active first;
 * folders follow the same way.
 */
export function threadGroups(
  threads: readonly Thread[],
  all: Readonly<Record<string, Thread>>,
  projects: readonly Project[],
): ThreadGroup[] {
  const known = new Map(projects.map((project) => [project.id, project]));
  const groups = new Map<string, ThreadGroup>();
  for (const project of projects) {
    groups.set(`project:${project.id}`, { kind: "project", key: `project:${project.id}`, project, threads: [], latest: project.created_at });
  }
  for (const thread of threads) {
    const root = rootOf(thread, all);
    const project = root.project ? known.get(root.project) : undefined;
    const key = project ? `project:${project.id}` : `folder:${folderOf(root)}`;
    let group = groups.get(key);
    if (!group) {
      group = { kind: "folder", key, folder: folderOf(root), threads: [], latest: 0 };
      groups.set(key, group);
    }
    group.threads.push(thread);
    group.latest = Math.max(group.latest, thread.updated_at);
  }
  const ordered = [...groups.values()].sort((a, b) => b.latest - a.latest);
  return [...ordered.filter((group) => group.kind === "project"), ...ordered.filter((group) => group.kind === "folder")];
}

export type Attention = "archived" | "failed" | "waiting" | "active" | "idle";

/** The one thing about a thread its row has to say. */
export function attentionOf(thread: Thread): Attention {
  if (thread.archived) return "archived";
  if ((thread.pending_interactions ?? []).length > 0) return "waiting";
  if (thread.status === "active") return "active";
  if (thread.status === "failed" || thread.last_turn_status === "failed") return "failed";
  return "idle";
}

/** Why a failed thread is shown as failed: its session ended, or only its last turn. */
export function failureText(thread: Thread): string {
  return thread.status === "failed" ? "The session failed" : "The last turn failed";
}

/** The time a card shows: how long the running turn has gone, or since the thread last changed. */
export function cardTime(thread: Thread, now: number): string {
  return attentionOf(thread) === "active" && thread.turn_started_at
    ? elapsed(now - thread.turn_started_at)
    : relativeTime(thread.updated_at, now);
}

export const attentionLabel: Record<Attention, string> = {
  archived: "Archived",
  failed: "Failed",
  waiting: "Waiting for your answer",
  active: "Working",
  idle: "Idle",
};

export const modeLabel: Record<PermissionMode, string> = {
  ask: "Ask every time",
  "accept-edits": "Accept edits",
  auto: "Auto review",
  full: "Full access",
};

/** Modes from the strictest to the loosest, as the daemon orders them. */
const MODE_ORDER: readonly PermissionMode[] = ["ask", "accept-edits", "auto", "full"];

/**
 * The modes a thread may be put in. A thread may not run looser than the thread
 * that started it; a parent without a mode allows its children `ask` and the
 * agent's own configuration, and nothing else (the daemon's `check_ceiling`).
 */
export function allowedModes(
  modes: readonly PermissionMode[],
  parent: Thread | undefined,
): { modes: PermissionMode[]; agentDefault: boolean } {
  if (!parent) return { modes: [...modes], agentDefault: true };
  const ceiling = MODE_ORDER.indexOf(parent.permission_mode ?? "ask");
  return {
    modes: modes.filter((mode) => MODE_ORDER.indexOf(mode) <= ceiling),
    agentDefault: !parent.permission_mode,
  };
}

/** One line saying what the agent last did, or is doing. */
export function activityText(thread: Thread): string {
  if (!thread.activity) return "";
  const { kind, detail, status } = thread.activity;
  // Paths inside the working directory read better without it.
  const path = relativePath(detail, thread.cwd);
  switch (kind) {
    case "agentMessage":
      return detail || (status === "pending" ? "Writing…" : "");
    case "reasoning":
      return status === "pending" ? "Thinking…" : "Thought it over";
    case "plan":
      return detail ? `Plan: ${detail}` : "Planning…";
    case "command":
      return `Run ${detail}`;
    case "fileChange":
      return `Edit ${tailPath(path, 48)}`;
    case "fileRead":
      return `Read ${tailPath(path, 48)}`;
    case "search":
      return `Search ${detail}`;
    case "tool":
    case "extension":
      return detail;
  }
}

/** How far up a chain of parents is followed: a guard, not a limit anyone reaches. */
const MAX_ANCESTORS = 32;

/** The thread a family of delegated threads started from: the one the user began. */
export function rootOf(thread: Thread, threads: Readonly<Record<string, Thread>>): Thread {
  let current = thread;
  for (let step = 0; step < MAX_ANCESTORS && current.parent; step++) {
    const parent = threads[current.parent];
    if (!parent) break;
    current = parent;
  }
  return current;
}

/** The threads a thread started, oldest first. */
export function childrenOf(threads: readonly Thread[], id: string): Thread[] {
  return threads.filter((thread) => thread.parent === id).sort((a, b) => a.created_at - b.created_at);
}

export type TreeRow = {
  thread: Thread;
  depth: number;
  /** Threads this one started, shown or not. */
  children: number;
  /** Its children are hidden under it. */
  collapsed: boolean;
  /** How many threads are hidden under a collapsed row, at every depth. */
  hidden: number;
  /** The most pressing state among the hidden ones, so a collapsed row still tells. */
  hiddenAttention: Attention | null;
};

/** Which state a collapsed family shows when its members differ. */
const URGENCY: readonly Attention[] = ["waiting", "failed", "active", "idle", "archived"];

/**
 * Threads as a list in which delegated threads sit under the thread that started
 * them, in the order they were started. Threads without a parent in the list come
 * first, newest first. The children of a thread in `collapsed` are left out, unless
 * `reveal` is among them: a selected thread is never hidden.
 */
export function threadTree(
  threads: readonly Thread[],
  collapsed: ReadonlySet<string> = new Set(),
  reveal?: string | null,
): TreeRow[] {
  const known = new Set(threads.map((thread) => thread.id));
  const byId = new Map(threads.map((thread) => [thread.id, thread]));
  // The ancestors of the revealed thread stay open.
  const open = new Set<string>();
  for (let id = reveal ? byId.get(reveal)?.parent : undefined, step = 0; id && step < MAX_ANCESTORS; step++) {
    open.add(id);
    id = byId.get(id)?.parent ?? undefined;
  }
  const descendants = (thread: Thread, depth: number): Thread[] =>
    depth >= MAX_ANCESTORS
      ? []
      : childrenOf(threads, thread.id).flatMap((child) => [child, ...descendants(child, depth + 1)]);

  const rows: TreeRow[] = [];
  const visit = (thread: Thread, depth: number) => {
    const children = childrenOf(threads, thread.id);
    const folded = children.length > 0 && collapsed.has(thread.id) && !open.has(thread.id);
    const hidden = folded ? descendants(thread, depth) : [];
    const states = new Set(hidden.map(attentionOf));
    rows.push({
      thread,
      depth,
      children: children.length,
      collapsed: folded,
      hidden: hidden.length,
      hiddenAttention: URGENCY.find((state) => states.has(state)) ?? null,
    });
    if (!folded && depth < MAX_ANCESTORS) for (const child of children) visit(child, depth + 1);
  };
  const roots = threads
    .filter((thread) => !thread.parent || !known.has(thread.parent))
    .sort((a, b) => b.created_at - a.created_at);
  for (const root of roots) visit(root, 0);
  return rows;
}

/** Dropped onto a thread: under it (`into`) or next to it, under its parent (`beside`). */
export type DropZone = "into" | "beside";

/** What dropping a thread somewhere would do. */
export type Drop = { parent: string | null } | { refused: string };

/**
 * Where a thread dragged in the sidebar lands: `target` is the thread it is
 * dropped on, or null for the top of its group. Null when nothing would change.
 * Threads move only within their group, and the daemon's rules are checked here
 * too, so a drop it would refuse says why before it is let go.
 */
export function dropOf(
  group: readonly Thread[],
  dragged: string,
  target: string | null,
  zone: DropZone,
): Drop | null {
  const byId = new Map(group.map((thread) => [thread.id, thread]));
  const thread = byId.get(dragged);
  if (!thread || (target !== null && !byId.has(target))) return null;
  const onto = target === null ? undefined : byId.get(target);
  let parent: string | null = onto ? (zone === "into" ? onto.id : (onto.parent ?? null)) : null;
  // A parent outside the group is not shown above it: next to such a thread is the top.
  if (parent !== null && !byId.has(parent)) parent = null;
  const current = thread.parent && byId.has(thread.parent) ? thread.parent : null;
  if (parent === current) return null;
  if (parent === null) return { parent };
  for (let id: string | null | undefined = parent, step = 0; id && step <= MAX_ANCESTORS; step++) {
    if (id === dragged) return { refused: "A thread cannot go under itself or a thread it started" };
    id = byId.get(id)?.parent;
  }
  const newParent = byId.get(parent);
  const { modes, agentDefault } = allowedModes(MODE_ORDER, newParent);
  const fits = thread.permission_mode ? modes.includes(thread.permission_mode) : agentDefault;
  if (!fits) {
    return { refused: `Runs looser than “${threadTitle(newParent)}” allows` };
  }
  return { parent };
}
