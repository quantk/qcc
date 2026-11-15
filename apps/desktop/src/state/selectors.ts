import { relativePath, tailPath } from "../util/format";
import type { PermissionMode, PluginInfo, Project, ProviderInfo, Thread } from "../api/protocol.gen";

/** Example plugins are left out unless asked for. */
export function visiblePlugins(plugins: readonly PluginInfo[], showExamples: boolean): readonly PluginInfo[] {
  return showExamples ? plugins : plugins.filter((plugin) => !plugin.example);
}

export function providersOf(plugins: readonly PluginInfo[]): ProviderInfo[] {
  return plugins.flatMap((plugin) => plugin.providers);
}

export function environmentsOf(plugins: readonly PluginInfo[]) {
  return plugins.flatMap((plugin) => plugin.environments);
}

export function findProvider(plugins: readonly PluginInfo[], id: string): ProviderInfo | undefined {
  return providersOf(plugins).find((provider) => provider.id === id);
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

export type TreeRow = { thread: Thread; depth: number; children: number };

/**
 * Threads as a list in which delegated threads sit under the thread that started
 * them, in the order they were started. Threads without a parent in the list come
 * first, newest first.
 */
export function threadTree(threads: readonly Thread[]): TreeRow[] {
  const known = new Set(threads.map((thread) => thread.id));
  const rows: TreeRow[] = [];
  const visit = (thread: Thread, depth: number) => {
    const children = childrenOf(threads, thread.id);
    rows.push({ thread, depth, children: children.length });
    if (depth < MAX_ANCESTORS) for (const child of children) visit(child, depth + 1);
  };
  const roots = threads
    .filter((thread) => !thread.parent || !known.has(thread.parent))
    .sort((a, b) => b.created_at - a.created_at);
  for (const root of roots) visit(root, 0);
  return rows;
}
