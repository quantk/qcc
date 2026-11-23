// Stepping through the open comments on a thread's changes in the order the diff
// shows them: by file as the changes list them, then down each file. A file whose
// diff is not read yet orders its comments by their line numbers, which is how
// they will come once it is read.

import type { LineComment } from "../api/protocol.gen";
import type { Placement } from "./comments";

/** A comment the bar steps to, with where it stands in the diff. */
export type CommentStop = {
  comment: LineComment;
  path: string;
  /** The file's place among the changed files. */
  file: number;
  /** Its place in the file: 0 above the lines (adrift), else one past the line index it is under. */
  at: number;
  /**
   * Where it is shown: under the line at `index`, above the file's lines, or not
   * known until the file's diff is read.
   */
  where: { kind: "line"; index: number } | { kind: "head" } | { kind: "unread" };
};

/**
 * The open comments of `comments` on the files `paths` (in the order of the
 * diff), each with its place from `placement`. Comments on files that are no
 * longer changed are left out: they are not in the diff to scroll to.
 */
export function commentStops(
  paths: readonly string[],
  comments: readonly LineComment[],
  placement: Pick<Placement, "byFile" | "adrift">,
): CommentStop[] {
  const fileAt = new Map(paths.map((path, index) => [path, index]));
  const lineOf = new Map<LineComment["id"], number>();
  for (const lines of placement.byFile.values()) {
    for (const [index, here] of lines) for (const comment of here) lineOf.set(comment.id, index);
  }
  const adrift = new Set<LineComment["id"]>();
  for (const here of placement.adrift.values()) for (const comment of here) adrift.add(comment.id);

  const stops: CommentStop[] = [];
  for (const comment of comments) {
    if (comment.resolved_at != null) continue;
    const file = fileAt.get(comment.path);
    if (file === undefined) continue;
    const index = lineOf.get(comment.id);
    if (index !== undefined) {
      stops.push({ comment, path: comment.path, file, at: index + 1, where: { kind: "line", index } });
    } else if (adrift.has(comment.id)) {
      stops.push({ comment, path: comment.path, file, at: 0, where: { kind: "head" } });
    } else {
      stops.push({ comment, path: comment.path, file, at: comment.line, where: { kind: "unread" } });
    }
  }
  // Stable: comments under the same line keep the order they are listed in.
  return stops.sort((a, b) => a.file - b.file || a.at - b.at);
}

/** Where a stop stands, enough to find the next one after it is gone. */
export type StopPlace = { id: LineComment["id"]; file: number; at: number };

/** The position of the comment `id` among `stops`, or -1. */
export function stopIndex(stops: readonly CommentStop[], id: LineComment["id"] | null | undefined): number {
  return id == null ? -1 : stops.findIndex((stop) => stop.comment.id === id);
}

/**
 * The stop after (`step` 1) or before (-1) `current`, going round at the ends.
 * With no current one, the first or the last. A current comment that is no
 * longer among the stops (resolved, deleted) is stepped from where it stood.
 */
export function stepStop(
  stops: readonly CommentStop[],
  current: StopPlace | null,
  step: 1 | -1,
): CommentStop | null {
  if (stops.length === 0) return null;
  const at = stopIndex(stops, current?.id);
  if (at >= 0) return stops[(at + step + stops.length) % stops.length]!;
  if (!current) return step === 1 ? stops[0]! : stops.at(-1)!;
  const after = (stop: CommentStop) => stop.file - current.file || stop.at - current.at;
  if (step === 1) return stops.find((stop) => after(stop) > 0) ?? stops[0]!;
  return stops.findLast((stop) => after(stop) < 0) ?? stops.at(-1)!;
}
