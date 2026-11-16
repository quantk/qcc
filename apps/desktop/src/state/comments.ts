// Where comments on a thread's changes go in its diff. A comment is left on a line
// number, but the agent keeps working: the line moves, or leaves the diff. The
// line's text, kept with the comment, finds it again.

import type { DiffSide, LineComment } from "../api/protocol.gen";
import type { DiffFile, DiffLine } from "./diff";

/** The side a diff line counts on and its number there; null for headers and notes. */
export function lineAnchor(line: DiffLine): { side: DiffSide; line: number } | null {
  if (line.kind === "del" && line.oldNo !== undefined) return { side: "old", line: line.oldNo };
  if ((line.kind === "add" || line.kind === "context") && line.newNo !== undefined) {
    return { side: "new", line: line.newNo };
  }
  return null;
}

export type Placement = {
  /** By the index of the line in its file's `lines` they are shown under. */
  byFile: ReadonlyMap<string, ReadonlyMap<number, LineComment[]>>;
  /** Comments on files of the diff whose line is no longer in it, by file. */
  adrift: ReadonlyMap<string, LineComment[]>;
  /** Comments on files that are no longer changed. */
  elsewhere: LineComment[];
};

/**
 * Put each comment under its line: the one at its number if the text still
 * matches, else the nearest line on the same side with the same text. A comment
 * whose line is found nowhere is adrift in its file, or elsewhere without one.
 */
export function placeComments(files: readonly DiffFile[], comments: readonly LineComment[]): Placement {
  const byPath = new Map(files.map((file) => [file.path, file]));
  const byFile = new Map<string, Map<number, LineComment[]>>();
  const adrift = new Map<string, LineComment[]>();
  const elsewhere: LineComment[] = [];
  const push = <K,>(map: Map<K, LineComment[]>, key: K, comment: LineComment) => {
    const list = map.get(key);
    if (list) list.push(comment);
    else map.set(key, [comment]);
  };
  for (const comment of comments) {
    const file = byPath.get(comment.path);
    if (!file) {
      elsewhere.push(comment);
      continue;
    }
    const index = findLine(file, comment);
    if (index === null) {
      push(adrift, comment.path, comment);
      continue;
    }
    let lines = byFile.get(comment.path);
    if (!lines) byFile.set(comment.path, (lines = new Map()));
    push(lines, index, comment);
  }
  return { byFile, adrift, elsewhere };
}

function findLine(file: DiffFile, comment: LineComment): number | null {
  const quote = comment.quote ?? null;
  let best: { index: number; distance: number } | null = null;
  file.lines.forEach((line, index) => {
    const anchor = lineAnchor(line);
    if (!anchor || anchor.side !== comment.side) return;
    if (anchor.line === comment.line && (quote === null || line.text === quote)) {
      best = { index, distance: -1 };
      return;
    }
    if (quote === null || line.text !== quote) return;
    const distance = Math.abs(anchor.line - comment.line);
    if (!best || distance < best.distance) best = { index, distance };
  });
  return (best as { index: number } | null)?.index ?? null;
}
