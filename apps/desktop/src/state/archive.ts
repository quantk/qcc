// What archiving a thread takes with it. The daemon archives the whole tree under
// the thread (`thread.archive` with `descendants`); this is what the app tells the
// user before and after: which threads go, which of them are working and will be
// stopped, which hold uncommitted work that will be discarded, and what stayed.

import type { ArchiveRefusal, Thread, ThreadArchiveResult, ThreadId } from "../api/protocol.gen";
import { threadTitle } from "./selectors";

/** A guard against a cycle of parents, not a limit anyone reaches. */
const MAX_DEPTH = 32;

/** A thread archiving would take along, and how far below the archived one it is. */
export type TreeMember = { thread: Thread; depth: number };

/**
 * The threads under `id` that are not archived yet: what the daemon archives with
 * it. Each parent comes before its children, siblings in the order they were
 * started. An archived thread in between is looked through, as the daemon does,
 * and a plugin's hidden threads count too: they go all the same.
 */
export function liveDescendants(threads: readonly Thread[], id: ThreadId): TreeMember[] {
  const byParent = new Map<string, Thread[]>();
  for (const thread of threads) {
    if (!thread.parent) continue;
    const siblings = byParent.get(thread.parent) ?? [];
    siblings.push(thread);
    byParent.set(thread.parent, siblings);
  }
  for (const siblings of byParent.values()) siblings.sort((a, b) => a.created_at - b.created_at);
  const out: TreeMember[] = [];
  const visit = (parent: string, depth: number) => {
    if (depth > MAX_DEPTH) return;
    for (const thread of byParent.get(parent) ?? []) {
      if (!thread.archived) out.push({ thread, depth });
      visit(thread.id, depth + 1);
    }
  };
  visit(id, 1);
  return out;
}

/** Whether archiving would stop a turn the thread is in. */
export function isWorking(thread: Thread): boolean {
  return thread.status === "active";
}

/** Whether the thread has a directory of its own whose uncommitted work archiving removes. */
export function hasWorkdir(thread: Thread): boolean {
  return thread.environment?.state === "ready";
}

/** What the confirmation says and does, given what is known of uncommitted work so far. */
export type TreeWarning = {
  title: string;
  /** Threads that would be stopped. */
  working: number;
  /** Threads whose uncommitted work would be discarded, the archived one included. */
  uncommitted: number;
  /** Still asking the environments: the confirmation waits for the answer. */
  checking: boolean;
  confirm: string;
  /** Archive with `force`: the user was told what uncommitted work is discarded. */
  force: boolean;
};

/**
 * The warning before archiving `root` with `members` under it. `uncommitted` holds
 * the ids of threads (`root` among them) found with uncommitted work, or null while
 * that is being found out.
 */
export function treeWarning(
  root: Thread,
  members: readonly TreeMember[],
  uncommitted: ReadonlySet<ThreadId> | null,
): TreeWarning {
  const count = members.length;
  const working = members.filter((member) => isWorking(member.thread)).length;
  const dirty = uncommitted === null ? 0 : [root, ...members.map((m) => m.thread)].filter((t) => uncommitted.has(t.id)).length;
  const others = `${count} ${count === 1 ? "thread" : "threads"} under it`;
  return {
    title: `Archive "${threadTitle(root)}" and ${others}?`,
    working,
    uncommitted: dirty,
    checking: uncommitted === null,
    confirm: dirty > 0 ? `Archive ${count + 1} and discard changes` : `Archive ${count + 1} threads`,
    force: dirty > 0,
  };
}

/** A thread of the tree that stayed, and why. */
export type Stayed = { id: ThreadId; title: string; reason: string };

/**
 * What stayed after an archive, or null if everything went. A refused thread says
 * why; a thread kept for one under it says that.
 */
export function stayedAfter(
  result: Pick<ThreadArchiveResult, "refused" | "kept">,
  threads: Readonly<Record<ThreadId, Thread>>,
): Stayed[] | null {
  const refused: readonly ArchiveRefusal[] = result.refused ?? [];
  const kept = result.kept ?? [];
  if (refused.length === 0 && kept.length === 0) return null;
  const title = (id: ThreadId) => (threads[id] ? threadTitle(threads[id]) : id);
  return [
    ...refused.map((refusal) => ({ id: refusal.thread_id, title: title(refusal.thread_id), reason: refusal.message })),
    ...kept.map((id) => ({ id, title: title(id), reason: "a thread under it stayed" })),
  ];
}
