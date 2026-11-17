// Highlights code away from the window, so a long diff never stalls it. Asked for a
// piece of code, it answers with its tokens a few hundred lines at a time; between
// those it reads its messages, so a piece no longer wanted is dropped mid-way.

import type { GrammarState, HighlighterCore } from "shiki/core";

import { createShiki, ensureLanguage, getLanguage } from "./shiki";
import type { HighlightAnswer, HighlightAsk, Token } from "./highlightProtocol";

/** Lines tokenized between two looks at the messages. */
const CHUNK = 200;
/** Lines longer than this are left plain: a minified line would take seconds. */
const MAX_LINE = 2000;

let shiki: Promise<HighlighterCore> | undefined;
const cancelled = new Set<number>();
const queue: Extract<HighlightAsk, { type: "highlight" }>[] = [];
let working = false;

const post = (answer: HighlightAnswer) => (self as unknown as Worker).postMessage(answer);
const pause = () => new Promise((resolve) => setTimeout(resolve, 0));

self.onmessage = ({ data }: MessageEvent<HighlightAsk>) => {
  if (data.type === "cancel") {
    cancelled.add(data.id);
    return;
  }
  queue.push(data);
  if (!working) void work();
};

async function work() {
  working = true;
  try {
    // Last asked, first done: that is what the user looks at now.
    for (let job = queue.pop(); job; job = queue.pop()) {
      if (cancelled.delete(job.id)) continue;
      await highlight(job.id, job.lang, job.text).catch(() => post({ id: job.id, from: 0, lines: [], done: true }));
    }
  } finally {
    working = false;
    cancelled.clear();
  }
}

async function highlight(id: number, language: string, text: string) {
  const lang = getLanguage(language);
  if (!lang || lang === "ansi") {
    post({ id, from: 0, lines: [], done: true });
    return;
  }
  shiki ??= createShiki();
  const highlighter = await shiki;
  await ensureLanguage(highlighter, lang);
  const lines = text.split("\n");
  let state: GrammarState | undefined;
  for (let from = 0; from < lines.length; from += CHUNK) {
    if (cancelled.delete(id)) return;
    const piece = lines.slice(from, from + CHUNK);
    const tokens = highlighter.codeToTokens(piece.join("\n"), {
      lang,
      themes: { light: "github-light", dark: "github-dark" },
      defaultColor: false,
      grammarState: state,
      tokenizeMaxLineLength: MAX_LINE,
    }).tokens;
    state = highlighter.getLastGrammarState(tokens);
    const answer: Token[][] = tokens.map((line) =>
      line.map((token) => [token.content, token.htmlStyle as Record<string, string> | undefined]),
    );
    const done = from + CHUNK >= lines.length;
    post({ id, from, lines: answer, done });
    if (!done) await pause();
  }
}
