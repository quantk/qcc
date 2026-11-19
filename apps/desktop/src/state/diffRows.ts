// The changes of a thread laid out as one list of rows, for a view that draws only
// the rows in sight: a file's header, its lines, what a view adds under a line, and
// a placeholder for a diff that is folded or not read yet.

import type { ChangedFile } from "../api/protocol.gen";
import { type FileDiff, isBinary, startsFolded } from "./changes";
import type { DiffFile } from "./diff";

/** Why a file shows a note instead of lines. */
export type FileNote = "folded" | "reading" | "binary" | "failed" | "empty" | "truncated";

export type DiffRow =
  | { kind: "file"; key: string; path: string; open: boolean }
  /** What the view shows above the lines (comments adrift in the file). */
  | { kind: "head"; key: string; path: string }
  | { kind: "line"; key: string; path: string; index: number }
  /** What the view shows under the line at `index`. */
  | { kind: "below"; key: string; path: string; index: number }
  | { kind: "note"; key: string; path: string; note: FileNote }
  /** The bottom edge of a file's box. */
  | { kind: "end"; key: string; path: string };

/** Pixels of one line of a diff: fixed, so the lines out of sight need no measuring. */
export const LINE_HEIGHT = 18;

/** A guess at a row's height before it is drawn; lines are exact. */
export function estimateRowHeight(row: DiffRow): number {
  switch (row.kind) {
    case "line":
      return LINE_HEIGHT;
    case "file":
      return 31;
    case "note":
      return 30;
    case "end":
      return 13;
    case "head":
    case "below":
      return 90;
  }
}

export type RowsInput = {
  files: readonly ChangedFile[];
  diffs: ReadonlyMap<string, FileDiff>;
  /** What the user opened or folded, over each file's default. */
  opened: ReadonlyMap<string, boolean>;
  /** Files with something to show above their lines. */
  heads?: ReadonlySet<string>;
  /** By file, the lines with something to show under them. */
  below?: ReadonlyMap<string, ReadonlySet<number>>;
};

/** The diff of a file as it can be shown now: read, or read before and being read again. */
export function shownFile(diff: FileDiff | undefined): DiffFile | undefined {
  if (diff?.state === "read") return diff.file;
  if (diff?.state === "reading") return diff.file;
  return undefined;
}

/** Whether a file is open: as the user left it, else folded only when large or generated. */
export function isOpen(file: ChangedFile, diff: FileDiff | undefined, opened: ReadonlyMap<string, boolean>): boolean {
  return opened.get(file.path) ?? !startsFolded(file, shownFile(diff));
}

export function diffRows({ files, diffs, opened, heads, below }: RowsInput): DiffRow[] {
  const rows: DiffRow[] = [];
  for (const changed of files) {
    const { path } = changed;
    const diff = diffs.get(path);
    const open = isOpen(changed, diff, opened);
    rows.push({ kind: "file", key: `f:${path}`, path, open });
    if (!open) {
      rows.push({ kind: "note", key: `n:${path}`, path, note: "folded" });
      rows.push({ kind: "end", key: `e:${path}`, path });
      continue;
    }
    const file = shownFile(diff);
    if (heads?.has(path) && file) rows.push({ kind: "head", key: `h:${path}`, path });
    if (isBinary(changed) || file?.binary) {
      rows.push({ kind: "note", key: `n:${path}`, path, note: "binary" });
    } else if (file) {
      const marks = below?.get(path);
      file.lines.forEach((_, index) => {
        rows.push({ kind: "line", key: `l:${path}:${index}`, path, index });
        if (marks?.has(index)) rows.push({ kind: "below", key: `b:${path}:${index}`, path, index });
      });
      if (file.lines.length === 0) rows.push({ kind: "note", key: `n:${path}`, path, note: "empty" });
      else if (diff?.state === "read" && diff.truncated) {
        rows.push({ kind: "note", key: `n:${path}`, path, note: "truncated" });
      }
    } else {
      const note = diff?.state === "failed" ? "failed" : "reading";
      rows.push({ kind: "note", key: `n:${path}`, path, note });
    }
    rows.push({ kind: "end", key: `e:${path}`, path });
  }
  return rows;
}

/** The open files with a row between `start` and `end` (indices into `rows`), in order. */
export function wantedPaths(rows: readonly DiffRow[], start: number, end: number): string[] {
  const paths: string[] = [];
  const seen = new Set<string>();
  for (let index = Math.max(0, start); index <= Math.min(rows.length - 1, end); index++) {
    const row = rows[index]!;
    const open =
      row.kind === "file" ? row.open : row.kind === "note" ? row.note !== "folded" : row.kind !== "end";
    if (!open || seen.has(row.path)) continue;
    seen.add(row.path);
    paths.push(row.path);
  }
  return paths;
}

/** The index of the first added or removed line of `file`, null when it has none. */
export function firstChange(file: DiffFile): number | null {
  const index = file.lines.findIndex((line) => line.kind === "add" || line.kind === "del");
  return index < 0 ? null : index;
}

/** Columns a line takes, tabs at `tabSize`. */
export function lineColumns(text: string, tabSize = 4): number {
  if (!text.includes("\t")) return text.length;
  let columns = 0;
  for (const char of text) columns = char === "\t" ? columns + tabSize - (columns % tabSize) : columns + 1;
  return columns;
}

const widest = new WeakMap<DiffFile, { columns: number; number: number }>();

/** The widest line of `file` in columns, and its largest line number. */
export function fileExtent(file: DiffFile): { columns: number; number: number } {
  let extent = widest.get(file);
  if (!extent) {
    extent = { columns: 0, number: 0 };
    for (const line of file.lines) {
      if (line.kind === "hunk" || line.kind === "note") continue;
      extent.columns = Math.max(extent.columns, lineColumns(line.text));
      extent.number = Math.max(extent.number, line.oldNo ?? 0, line.newNo ?? 0);
    }
    widest.set(file, extent);
  }
  return extent;
}

/**
 * How wide the code of the open files is, in columns, and how many digits their
 * line numbers take: the whole diff scrolls sideways as one, so every line's
 * background reaches as far as the longest.
 */
export function diffExtent(rows: readonly DiffRow[], fileOf: (path: string) => DiffFile | undefined) {
  let columns = 0;
  let number = 0;
  for (const row of rows) {
    if (row.kind !== "file" || !row.open) continue;
    const file = fileOf(row.path);
    if (!file) continue;
    const extent = fileExtent(file);
    columns = Math.max(columns, extent.columns);
    number = Math.max(number, extent.number);
  }
  return { columns, digits: Math.max(3, String(number).length) };
}
