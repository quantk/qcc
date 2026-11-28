// Fuzzy matching in the manner of fzf: the query's characters must appear in the
// text in order; an alignment scores higher where they run together, start a
// word or a path segment, or fall in the last segment (a file's own name). The
// daemon matches paths the same way (`crates/qcc-daemon/src/paths.rs`); this
// copy ranks what the window already holds, such as skills.

export type FuzzyMatch = {
  score: number;
  /** Where the query's characters matched, counted in characters (code points) of the text. */
  positions: number[];
};

const SCORE_MATCH = 16;
const GAP_START = -3;
const GAP_EXTENSION = -1;
const BONUS_SEGMENT = 10;
const BONUS_WORD = 8;
const BONUS_CAMEL = 7;
const BONUS_CONSECUTIVE = 4;
const BONUS_NAME = 3;
const FIRST_MULTIPLIER = 2;
const NONE = -1e9;

/** Read a cell of a table the loops below keep within bounds. */
const at = (table: ArrayLike<number>, index: number): number => table[index] as number;

const isUpper = (c: string) => c !== c.toLowerCase() && c === c.toUpperCase();
const isLower = (c: string) => c !== c.toUpperCase() && c === c.toLowerCase();
const isDigit = (c: string) => c >= "0" && c <= "9";

function boundary(previous: string | undefined, current: string): number {
  if (previous === undefined || previous === "/") return BONUS_SEGMENT;
  if (previous === "-" || previous === "_" || previous === "." || previous === " ") return BONUS_WORD;
  if (isLower(previous) && isUpper(current)) return BONUS_CAMEL;
  if (!isDigit(previous) && isDigit(current)) return BONUS_CAMEL;
  return 0;
}

/**
 * Score `query` against `text`; null when its characters are not all there in
 * order. Smart case: a query with a capital letter matches case-sensitively.
 * Whitespace in the query is ignored.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  const wanted = query.replace(/\s+/g, "");
  if (wanted === "") return { score: 0, positions: [] };
  const sensitive = [...wanted].some(isUpper);
  // Folded a character at a time, so positions stay those of the text.
  const fold = (c: string) => {
    if (sensitive) return c;
    const lower = c.toLowerCase();
    return [...lower].length === 1 ? lower : c;
  };
  const q = [...wanted].map(fold);
  const chars = [...text];
  const folded = chars.map(fold);
  const m = q.length;
  const n = folded.length;

  let next = 0;
  for (let j = 0; j < n && next < m; j++) if (folded[j] === q[next]) next++;
  if (next < m) return null;

  const nameStart = chars.lastIndexOf("/") + 1;
  const bonus = new Float64Array(n);
  for (let j = 0; j < n; j++) bonus[j] = (j >= nameStart ? BONUS_NAME : 0) + boundary(chars[j - 1], chars[j] as string);

  const score = new Float64Array(m * n).fill(NONE);
  const from = new Int32Array(m * n);
  const chunk = new Float64Array(m * n);
  for (let i = 0; i < m; i++) {
    let gap = NONE;
    let gapFrom = 0;
    for (let j = i; j < n; j++) {
      if (i > 0 && j >= 2) {
        const opened = at(score, (i - 1) * n + j - 2) + GAP_START;
        const extended = gap + GAP_EXTENSION;
        if (opened >= extended) {
          gap = opened;
          gapFrom = j - 2;
        } else gap = extended;
      }
      if (folded[j] !== q[i]) continue;
      const cell = i * n + j;
      if (i === 0) {
        score[cell] = SCORE_MATCH + at(bonus, j) * FIRST_MULTIPLIER;
        chunk[cell] = at(bonus, j);
        continue;
      }
      const before = (i - 1) * n + j - 1;
      const run =
        at(score, before) > NONE / 2 ? at(score, before) + Math.max(at(bonus, j), at(chunk, before), BONUS_CONSECUTIVE) : NONE;
      const jump = gap > NONE / 2 ? gap + at(bonus, j) : NONE;
      if (run <= NONE / 2 && jump <= NONE / 2) continue;
      if (jump > run) {
        score[cell] = SCORE_MATCH + jump;
        from[cell] = gapFrom;
        chunk[cell] = at(bonus, j);
      } else {
        score[cell] = SCORE_MATCH + run;
        from[cell] = j - 1;
        chunk[cell] = Math.max(at(chunk, before), at(bonus, j));
      }
    }
  }

  const last = (m - 1) * n;
  let end = -1;
  for (let j = 0; j < n; j++) if (at(score, last + j) > NONE / 2 && (end < 0 || at(score, last + j) > at(score, last + end))) end = j;
  if (end < 0) return null;
  const positions = new Array<number>(m);
  let j = end;
  for (let i = m - 1; i >= 0; i--) {
    positions[i] = j;
    j = at(from, i * n + j);
  }
  return { score: at(score, last + end), positions };
}

/**
 * The items that match `query` best, best first; ties go to the shorter text,
 * then the earlier item. An empty query keeps every item in its order.
 */
export function fuzzyRank<T>(items: readonly T[], query: string, text: (item: T) => string): { item: T; match: FuzzyMatch }[] {
  if (query.trim() === "") return items.map((item) => ({ item, match: { score: 0, positions: [] } }));
  const ranked: { item: T; match: FuzzyMatch; index: number; length: number }[] = [];
  items.forEach((item, index) => {
    const value = text(item);
    const match = fuzzyMatch(query, value);
    if (match) ranked.push({ item, match, index, length: [...value].length });
  });
  ranked.sort((a, b) => b.match.score - a.match.score || a.length - b.length || a.index - b.index);
  return ranked.map(({ item, match }) => ({ item, match }));
}
