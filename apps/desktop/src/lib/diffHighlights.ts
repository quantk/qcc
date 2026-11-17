// Highlighting for long diffs: only the lines in view are asked for, a hunk side
// at a time, of a worker that does the work away from the window. What comes back
// is kept with the parsed file, so a file scrolled away and back, or read again
// unchanged, is not highlighted twice.

import { type DiffFile, type HighlightRun, highlightRuns } from "../state/diff";
import { languageOfPath } from "../state/languages";
import type { HighlightAnswer, HighlightAsk, Token } from "./highlightProtocol";

/** Files longer than this stay plain: they are lockfiles and data more than code. */
const HIGHLIGHT_UP_TO = 10_000;

type FileHighlight = {
  language: string | null;
  runs: HighlightRun[];
  /** For each line of the file, the runs it is in (a context line is on both sides). */
  runsOf: number[][];
  tokens: (Token[] | undefined)[];
  /** Runs asked for or done. */
  asked: Set<number>;
};

type Job = { file: DiffFile; run: number; owners: Set<string> };

/** Lines of one file a view shows: `from` to `to`, by index in `file.lines`. */
export type InView = { file: DiffFile; from: number; to: number };

export class DiffHighlights {
  private files = new WeakMap<DiffFile, FileHighlight>();
  private worker: Worker | null | undefined;
  private jobs = new Map<number, Job>();
  private nextId = 1;
  private listeners = new Set<() => void>();
  private version = 0;
  private notifying = false;

  getVersion = () => this.version;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** The tokens of a line, once they have come. */
  tokens(file: DiffFile, index: number): Token[] | undefined {
    return this.files.get(file)?.tokens[index];
  }

  /**
   * What `owner` (one view) shows now. Runs in view are asked for; runs only this
   * view wanted and no longer does are called off, so a fast scroll does not leave
   * a queue of work behind it.
   */
  show(owner: string, inView: readonly InView[]) {
    for (const { file, from, to } of inView) {
      const highlight = this.of(file);
      if (!highlight?.language) continue;
      for (let index = from; index <= to; index++) {
        for (const run of highlight.runsOf[index] ?? []) {
          this.ask(file, highlight, run, owner);
        }
      }
    }
    for (const [id, job] of this.jobs) {
      const highlight = this.files.get(job.file)!;
      const stillWanted = inView.some(
        ({ file, from, to }) =>
          file === job.file && highlight.runs[job.run]!.indices.some((index) => index >= from && index <= to),
      );
      if (stillWanted) continue;
      job.owners.delete(owner);
      if (job.owners.size > 0) continue;
      this.jobs.delete(id);
      highlight.asked.delete(job.run);
      this.post({ type: "cancel", id });
    }
  }

  private of(file: DiffFile): FileHighlight | undefined {
    let highlight = this.files.get(file);
    if (!highlight) {
      const language = file.binary || file.lines.length > HIGHLIGHT_UP_TO ? null : languageOfPath(file.path);
      const runs = language ? highlightRuns(file) : [];
      const runsOf: number[][] = [];
      runs.forEach((run, at) => {
        for (const index of run.indices) (runsOf[index] ??= []).push(at);
      });
      highlight = { language, runs, runsOf, tokens: [], asked: new Set() };
      this.files.set(file, highlight);
    }
    return highlight;
  }

  private ask(file: DiffFile, highlight: FileHighlight, run: number, owner: string) {
    if (highlight.asked.has(run)) {
      for (const job of this.jobs.values()) if (job.file === file && job.run === run) job.owners.add(owner);
      return;
    }
    if (!this.ensureWorker()) return;
    highlight.asked.add(run);
    const id = this.nextId++;
    this.jobs.set(id, { file, run, owners: new Set([owner]) });
    this.post({ type: "highlight", id, lang: highlight.language!, text: highlight.runs[run]!.text });
  }

  private ensureWorker(): boolean {
    if (this.worker === undefined) {
      try {
        this.worker =
          typeof Worker === "undefined"
            ? null
            : new Worker(new URL("./highlight.worker.ts", import.meta.url), { type: "module" });
        this.worker?.addEventListener("message", (event: MessageEvent<HighlightAnswer>) => this.answered(event.data));
      } catch {
        this.worker = null;
      }
    }
    return this.worker !== null;
  }

  private post(ask: HighlightAsk) {
    this.worker?.postMessage(ask);
  }

  private answered({ id, from, lines, done }: HighlightAnswer) {
    const job = this.jobs.get(id);
    if (!job) return;
    if (done) this.jobs.delete(id);
    const highlight = this.files.get(job.file);
    const run = highlight?.runs[job.run];
    if (!highlight || !run) return;
    lines.forEach((tokens, at) => {
      const index = run.indices[from + at];
      if (index !== undefined) highlight.tokens[index] = tokens;
    });
    this.changed();
  }

  /** Answers come in bursts; views redraw once a frame for them. */
  private changed() {
    if (this.notifying) return;
    this.notifying = true;
    const run = () => {
      this.notifying = false;
      this.version++;
      for (const listener of this.listeners) listener();
    };
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    else queueMicrotask(run);
  }
}

/** The one highlighter of diffs in the window. */
export const diffHighlights = new DiffHighlights();
