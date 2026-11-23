// Quoting text from the timeline into the composer: what the message box holds
// once a quote of the selection is added to what the user had typed.

/** The selection as a markdown quote: every line behind `> `, blank lines too. */
export function quoteMarkdown(selection: string): string {
  const lines = selection
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd());
  // Blank lines at the edges of a selection are where the drag happened to start or
  // end, not part of what is quoted.
  while (lines.length > 0 && lines[0] === "") lines.shift();
  while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line) => (line === "" ? ">" : `> ${line}`)).join("\n");
}

/**
 * The composer's text with a quote of `selection` after it: a blank line apart from
 * what was typed, and a blank line after the quote, where the caret goes. A selection
 * of nothing but whitespace leaves the text as it was.
 */
export function appendQuote(current: string, selection: string): string {
  const quote = quoteMarkdown(selection);
  if (quote === "") return current;
  const before = current.trimEnd();
  return `${before === "" ? "" : `${before}\n\n`}${quote}\n\n`;
}
