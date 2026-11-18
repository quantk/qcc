// How much of a long output the timeline shows before the reader asks for all of it.

/** The start of an output that is shown at first. */
export type Preview = {
  text: string;
  /** Lines in the whole output. */
  lines: number;
  /** Whether `text` leaves anything out. */
  clipped: boolean;
};

/**
 * The first `lines` lines of `text`, and no more than `chars` characters of them, so
 * that one very long line does not wrap into a wall either. An output only a little
 * longer than that is shown whole: a button that reveals two more lines is not worth
 * its place.
 */
export function previewText(text: string, limits: { lines: number; chars: number }): Preview {
  // A trailing newline is not a line of its own.
  const whole = text.endsWith("\n") ? text.slice(0, -1) : text;
  const all = whole.split("\n");
  const total = all.length;
  if (total <= limits.lines + Math.ceil(limits.lines / 4) && whole.length <= limits.chars * 1.25) {
    return { text, lines: total, clipped: false };
  }
  const kept: string[] = [];
  let budget = limits.chars;
  for (const line of all) {
    if (kept.length === limits.lines || budget <= 0) break;
    kept.push(line.length > budget ? line.slice(0, budget) : line);
    budget -= line.length + 1;
  }
  return { text: kept.join("\n"), lines: total, clipped: true };
}
