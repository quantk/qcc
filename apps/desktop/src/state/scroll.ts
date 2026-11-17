// How the virtualized timeline guesses and settles its scroll. Pure: the view
// measures, these decide.

import type { Entry } from "./timeline";

/** Pixels of one line of body text, and of a folded trace line. */
const LINE = 24;
const TRACE = 32;
/** The row of attachments under a user message. */
const ATTACHMENTS = 88;
/** Characters on one line of the timeline at its widest, roughly. */
const LINE_CHARS = 95;

function textHeight(text: string): number {
  let lines = 0;
  for (const paragraph of text.split("\n")) lines += Math.max(1, Math.ceil(paragraph.length / LINE_CHARS));
  return lines * LINE;
}

/**
 * A guess at an entry's height before it is drawn. Only entries out of view are
 * guessed, and each is measured once it is drawn; a closer guess just moves the
 * scrollbar less while that happens.
 */
export function estimateEntryHeight(entry: Entry): number {
  switch (entry.kind) {
    case "user":
      return textHeight(entry.text) + 20 + entry.notes.length * 20 + (entry.attachments.length > 0 ? ATTACHMENTS : 0);
    case "notice":
    case "conversation":
      return TRACE;
    case "interaction":
      return entry.resolution ? 56 : 140;
    case "item": {
      const { item } = entry;
      switch (item.type) {
        case "agentMessage":
          return textHeight(item.text) + 16;
        case "plan":
          return TRACE + textHeight(item.text);
        case "fileChange": {
          // A short diff is shown open; count its lines.
          const lines = item.diff ? item.diff.split("\n").length : 0;
          return lines > 0 && lines <= 34 ? TRACE + lines * 20 : TRACE;
        }
        default:
          return TRACE;
      }
    }
  }
}

/**
 * The user message that starts a new turn, if `next` brought one: the last user
 * message of `next` when `previous` did not have it. Nothing is new to a view that
 * had nothing: the first message of a thread is at the top anyway, and a timeline
 * that arrives whole is opened at its end.
 */
export function newTurnKey(previous: readonly Entry[], next: readonly Entry[]): string | null {
  if (previous.length === 0) return null;
  const last = lastUserKey(next);
  if (last === null || last === lastUserKey(previous)) return null;
  return previous.some((entry) => entry.key === last) ? null : last;
}

function lastUserKey(entries: readonly Entry[]): string | null {
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index]!;
    if (entry.kind === "user") return entry.key;
  }
  return null;
}

/**
 * Empty space below the content that lets a new turn's message sit at the top of
 * the view while its answer is shorter than the view. It shrinks as the answer
 * grows and is gone once the answer fills the view, so from then on the view
 * follows the end as usual.
 */
export function turnRoom(options: {
  /** Where the anchored message starts, from the top of the content. */
  anchorStart: number;
  /** How much of the entry above it stays in view. */
  peek: number;
  /** Where the content ends, without this room. */
  contentEnd: number;
  viewportHeight: number;
}): number {
  const top = Math.max(0, options.anchorStart - options.peek);
  return Math.max(0, Math.ceil(top + options.viewportHeight - options.contentEnd));
}
