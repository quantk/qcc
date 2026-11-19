// Which part of a thread's work the Changes view shows: all of it, what is not
// committed yet, or a run of its commits. The picker lists the uncommitted work as
// row 0 and the commits after it, newest first; a choice is a run of rows.

import type { ChangeScope, Revision } from "../api/protocol.gen";

export const ALL: ChangeScope = { kind: "all" };

/** Rows `newest` to `oldest` of the picker, both included; row 0 is the uncommitted work. */
export type Span = { newest: number; oldest: number };

/** The run of rows from the one clicked first to the one shift-clicked. */
export function spanBetween(anchor: number, row: number): Span {
  return { newest: Math.min(anchor, row), oldest: Math.max(anchor, row) };
}

/** What a run of rows shows. All the rows are all the work; row 0 alone, the uncommitted. */
export function scopeOfSpan(span: Span, revisions: readonly Revision[]): ChangeScope {
  const { newest, oldest } = span;
  if (newest === 0 && oldest === 0) return { kind: "unsaved" };
  if (newest === 0 && oldest >= revisions.length) return ALL;
  const scope: ChangeScope = { kind: "revisions" };
  // Row `n` is commit `n - 1`; the changes start after the commit below the run.
  const from = revisions[oldest]?.id;
  if (from) scope.from = from;
  if (newest > 0) scope.to = revisions[newest - 1]!.id;
  return scope;
}

/** The rows a scope covers, or null when it names a commit no longer listed. */
export function spanOfScope(scope: ChangeScope, revisions: readonly Revision[]): Span | null {
  if (scope.kind === "all") return { newest: 0, oldest: revisions.length };
  if (scope.kind === "unsaved") return { newest: 0, oldest: 0 };
  const row = (id: string) => revisions.findIndex((revision) => revision.id === id);
  const newest = scope.to ? row(scope.to) + 1 : 0;
  const oldest = scope.from ? row(scope.from) : revisions.length;
  if ((scope.to && newest === 0) || oldest < 0 || oldest < Math.max(newest, 1)) return null;
  return { newest, oldest };
}

/**
 * Whether a scope still means something once the commits are read again: the
 * agent may have amended or rebased the ones it named.
 */
export function scopeHolds(scope: ChangeScope, revisions: readonly Revision[]): boolean {
  return spanOfScope(scope, revisions) !== null;
}

export function sameScope(a: ChangeScope, b: ChangeScope): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind !== "revisions" || b.kind !== "revisions") return true;
  return (a.from ?? null) === (b.from ?? null) && (a.to ?? null) === (b.to ?? null);
}

export function shortId(id: string): string {
  return id.slice(0, 7);
}

/** What the picker's button says about a scope. */
export function scopeLabel(scope: ChangeScope, revisions: readonly Revision[]): string {
  if (scope.kind === "all") return "All changes";
  if (scope.kind === "unsaved") return "Uncommitted";
  const span = spanOfScope(scope, revisions);
  if (!span) return "Some commits";
  const commits = span.oldest - Math.max(span.newest, 1) + 1;
  if (span.newest === 0) return commits === 1 ? "Last commit and uncommitted" : `${commits} commits and uncommitted`;
  if (commits === 1) {
    const revision = revisions[span.newest - 1]!;
    return `${shortId(revision.id)} ${revision.title}`;
  }
  return `${commits} commits`;
}
