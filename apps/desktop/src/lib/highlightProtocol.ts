// What the window and the highlighting worker say to each other.

/** A token: its text and the style that colors it in the light and the dark scheme. */
export type Token = [content: string, style: Record<string, string> | undefined];

export type HighlightAsk =
  | { type: "highlight"; id: number; lang: string; text: string }
  | { type: "cancel"; id: number };

/** The tokens of lines `from` on of the piece asked for as `id`; `done` with its last lines. */
export type HighlightAnswer = { id: number; from: number; lines: Token[][]; done: boolean };
