// Completion in the composer: which word under the caret asks for it (`/skill`,
// `@path`) and what choosing an entry does to the text. The lists themselves come
// from modules (`UiRegistry.composerCompletion`).

/** The word being completed: from its trigger character up to the caret. */
export type CompletionToken = {
  trigger: string;
  /** Where the trigger character is. */
  start: number;
  /** The caret: the token ends here. */
  end: number;
  /** What is typed after the trigger. */
  query: string;
};

/**
 * The token the caret ends, if it starts with one of `triggers` at the start of
 * the text or after whitespace, and holds no whitespace itself.
 */
export function completionToken(text: string, caret: number, triggers: readonly string[]): CompletionToken | null {
  if (caret < 0 || caret > text.length) return null;
  let start = caret;
  while (start > 0 && !/\s/.test(text.charAt(start - 1))) start--;
  if (start === caret) return null;
  const trigger = text.charAt(start);
  if (!triggers.includes(trigger)) return null;
  return { trigger, start, end: caret, query: text.slice(start + 1, caret) };
}

/** The text with the token replaced by `insert`, and where the caret goes. */
export function applyCompletion(text: string, token: CompletionToken, insert: string): { text: string; caret: number } {
  // A space already after the token is not doubled.
  const after = text.slice(token.end);
  const insertion = insert.endsWith(" ") && /^\s/.test(after) ? insert.slice(0, -1) : insert;
  const caret = token.start + insertion.length + (insertion === insert ? 0 : 1);
  return { text: text.slice(0, token.start) + insertion + after, caret };
}

/** How a path is written after `@`: quoted when it has whitespace in it. */
export function mentionPath(path: string, dir: boolean): string {
  const written = dir ? `${path}/` : path;
  return /\s/.test(written) ? `@"${written}"` : `@${written}`;
}
