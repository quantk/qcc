// Folding the sidebar's groups: a project, or a folder of threads in no project,
// shows only its heading, and the heading says what the hidden threads need.

import type { Project, Thread } from "../api/protocol.gen";
import { type Seen, isUnread } from "./reads";
import { type TreeRow, attentionOf, threadGroups } from "./selectors";

/** What a folded group's heading says about the threads under it. */
export type GroupSummary = {
  threads: number;
  waiting: number;
  active: number;
  failed: number;
  /** Idle threads with a reply the user has not read. */
  unread: number;
};

export function groupSummary(threads: readonly Thread[], seen: Seen): GroupSummary {
  const summary: GroupSummary = { threads: threads.length, waiting: 0, active: 0, failed: 0, unread: 0 };
  for (const thread of threads) {
    const attention = attentionOf(thread);
    if (attention === "waiting") summary.waiting++;
    else if (attention === "active") summary.active++;
    else if (attention === "failed") summary.failed++;
    else if (attention === "idle" && isUnread(thread, seen)) summary.unread++;
  }
  return summary;
}

/** The summary in words, for a tooltip and a screen reader. */
export function summaryText(summary: GroupSummary): string {
  const parts = [`${summary.threads} ${summary.threads === 1 ? "thread" : "threads"}`];
  if (summary.waiting > 0) parts.push(`${summary.waiting} waiting for your answer`);
  if (summary.failed > 0) parts.push(`${summary.failed} failed`);
  if (summary.active > 0) parts.push(`${summary.active} working`);
  if (summary.unread > 0) parts.push(`${summary.unread} with a new reply`);
  return parts.join(", ");
}

/**
 * The rows a group shows. A folded group shows none of its threads but the
 * selected one: the thread the user is in stays in sight, as it does under a
 * folded parent, without undoing the fold they chose. It shows on its own,
 * without the family lines of a tree it is no longer drawn in.
 */
export function visibleRows(rows: readonly TreeRow[], folded: boolean, selected: string | null): TreeRow[] {
  if (!folded) return [...rows];
  const row = rows.find((candidate) => candidate.thread.id === selected);
  return row ? [{ ...row, depth: 0, children: 0, collapsed: false, hidden: 0, hiddenAttention: null }] : [];
}

/** The keys with `keys` folded or unfolded. */
export function withFolded(folded: ReadonlySet<string>, keys: readonly string[], fold: boolean): Set<string> {
  const next = new Set(folded);
  for (const key of keys) {
    if (fold) next.add(key);
    else next.delete(key);
  }
  return next;
}

/**
 * The keys of every group there is: every project, and the folder of every
 * thread outside one, archived and plugin threads too, so a fold is not
 * forgotten while its threads are only out of view.
 */
export function groupKeys(threads: Readonly<Record<string, Thread>>, projects: readonly Project[]): Set<string> {
  return new Set(threadGroups(Object.values(threads), threads, projects).map((group) => group.key));
}

/** The folded keys whose groups still exist: a gone project or folder is not remembered. */
export function pruneFolded(folded: ReadonlySet<string>, existing: ReadonlySet<string>): Set<string> {
  return new Set([...folded].filter((key) => existing.has(key)));
}
