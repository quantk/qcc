// A thread's changes as a view reads them: the list of changed files at once, and
// the diff of each file only when it is in view. A large change (hundreds of files,
// a lockfile of tens of thousands of lines) costs what the screen shows, not what
// the agent did.

import type { ChangedFile, EnvironmentChanges, ThreadDiffParams, ThreadDiffResult } from "../api/protocol.gen";
import { type DiffFile, parseDiff } from "./diff";

/** Files with more changed lines than this start folded; their diff is read when opened. */
export const FOLD_ABOVE = 400;

/** Files read in one request: enough for a screen and what lies just past it. */
export const BATCH = 24;

/** Lockfiles and build output: long, written by tools, rarely worth reading. */
const GENERATED = [
  /(^|\/)(package-lock\.json|npm-shrinkwrap\.json|yarn\.lock|pnpm-lock\.yaml|bun\.lockb?|deno\.lock)$/,
  /(^|\/)(Cargo\.lock|flake\.lock|go\.sum|poetry\.lock|uv\.lock|Pipfile\.lock|Gemfile\.lock)$/,
  /(^|\/)(composer\.lock|mix\.lock|Podfile\.lock|pubspec\.lock|packages\.lock\.json|gradle\.lockfile)$/,
  /\.min\.(js|css)$/,
  /\.(js|css)\.map$/,
];

export function isGenerated(path: string): boolean {
  return GENERATED.some((pattern) => pattern.test(path));
}

/** Git counts no lines in a binary file; a new file is not counted at all. */
export function isBinary(file: ChangedFile): boolean {
  return file.kind !== "untracked" && (file.additions === null || file.additions === undefined);
}

/** Changed lines by the summary; unknown (null) for new files git does not track yet. */
export function changedLines(file: ChangedFile): number | null {
  if (file.additions === null || file.additions === undefined) return null;
  return file.additions + (file.deletions ?? 0);
}

/**
 * Whether a file is shown folded until the user opens it: generated files, and
 * those too long to read in passing. A new file is measured once its diff is read.
 */
export function startsFolded(file: ChangedFile, diff?: DiffFile): boolean {
  if (isGenerated(file.path)) return true;
  const lines = changedLines(file) ?? diff?.lines.length ?? 0;
  return lines > FOLD_ABOVE;
}

/** The diff of one file, as far as it is read. */
export type FileDiff =
  /** Not read, or to be read again on its own (it went missing from a shared answer). */
  | { state: "unread"; alone?: boolean }
  /** Being read; `file` is what was shown before, kept until the new one comes. */
  | { state: "reading"; file?: DiffFile; truncated?: boolean }
  /** `stale`: the agent may have changed it since; shown, and read again when in view. */
  | { state: "read"; file: DiffFile; truncated: boolean; stale: boolean }
  | { state: "failed"; error: string };

export type ChangesState = {
  changes: EnvironmentChanges | null;
  /** The list could not be read. */
  error: string | null;
  /** The list is being read. */
  loading: boolean;
  diffs: ReadonlyMap<string, FileDiff>;
};

export const EMPTY: ChangesState = { changes: null, error: null, loading: false, diffs: new Map() };

/**
 * A new list of changed files: diffs of files no longer changed are dropped, and
 * the rest are kept on screen but marked stale, since a file can change and keep
 * its counts. Unchanged state is returned as it was.
 */
export function withChanges(state: ChangesState, changes: EnvironmentChanges): ChangesState {
  const paths = new Set(changes.files.map((file) => file.path));
  const diffs = new Map<string, FileDiff>();
  for (const [path, diff] of state.diffs) {
    if (!paths.has(path)) continue;
    if (diff.state === "read") diffs.set(path, diff.stale ? diff : { ...diff, stale: true });
    else if (diff.state === "failed") diffs.set(path, { state: "unread" });
    else diffs.set(path, diff);
  }
  return { ...state, changes, error: null, loading: false, diffs };
}

/** Whether `path`'s diff has to be read before it can be shown as it is now. */
function needsReading(diff: FileDiff | undefined): boolean {
  if (!diff) return true;
  return diff.state === "unread" || (diff.state === "read" && diff.stale);
}

/**
 * Which of `wanted` (files in view, open, in the order shown) to read next, as one
 * request: those not read or stale, up to `BATCH`. A file that went missing from a
 * shared answer is asked for on its own. Binary files have nothing to read.
 */
export function nextBatch(state: ChangesState, wanted: readonly string[]): string[] {
  const byPath = new Map(state.changes?.files.map((file) => [file.path, file]));
  const batch: string[] = [];
  for (const path of wanted) {
    const file = byPath.get(path);
    if (!file || isBinary(file)) continue;
    const diff = state.diffs.get(path);
    if (!needsReading(diff)) continue;
    if (diff?.state === "unread" && diff.alone) {
      if (batch.length === 0) return [path];
      continue;
    }
    batch.push(path);
    if (batch.length >= BATCH) break;
  }
  return batch;
}

/** Mark `paths` as being read, keeping what each showed. */
export function reading(state: ChangesState, paths: readonly string[]): ChangesState {
  const diffs = new Map(state.diffs);
  for (const path of paths) {
    const diff = diffs.get(path);
    diffs.set(
      path,
      diff?.state === "read" ? { state: "reading", file: diff.file, truncated: diff.truncated } : { state: "reading" },
    );
  }
  return { ...state, diffs };
}

/** Whether two parses of a file say the same, so the one on screen can stay. */
export function sameFile(a: DiffFile, b: DiffFile): boolean {
  if (a === b) return true;
  if (a.path !== b.path || a.status !== b.status || a.binary !== b.binary || a.lines.length !== b.lines.length) {
    return false;
  }
  for (let index = 0; index < a.lines.length; index++) {
    const x = a.lines[index]!;
    const y = b.lines[index]!;
    if (x.kind !== y.kind || x.text !== y.text || x.oldNo !== y.oldNo || x.newNo !== y.newNo) return false;
  }
  return true;
}

function emptyFile(path: string, status: DiffFile["status"]): DiffFile {
  return { path, status, binary: false, additions: 0, deletions: 0, lines: [] };
}

const statusOf: Record<ChangedFile["kind"], DiffFile["status"]> = {
  added: "added",
  untracked: "added",
  deleted: "deleted",
  modified: "modified",
};

/**
 * The answer to reading `paths`. Each file found takes its diff, the very object
 * shown before when nothing in it changed, so a view does not draw or highlight it
 * again. A file the answer has nothing for has no textual change (a mode, say);
 * one missing from an answer that was cut short, or that git named differently
 * (quoted, escaped), is read again on its own.
 */
export function withDiff(state: ChangesState, paths: readonly string[], result: ThreadDiffResult): ChangesState {
  const parsed = parseDiff(result.diff);
  const found = new Map(parsed.map((file) => [file.path, file]));
  const last = parsed.at(-1);
  const kinds = new Map(state.changes?.files.map((file) => [file.path, file.kind]));
  const diffs = new Map(state.diffs);
  const single = paths.length === 1;
  for (const path of paths) {
    const before = diffs.get(path);
    // Not asked for any more: the list changed while this was read.
    if (before?.state !== "reading") continue;
    let file = found.get(path) ?? (single ? parsed[0] : undefined);
    if (!file) {
      if (paths.length > 1 && (result.truncated || parsed.length > 0)) {
        diffs.set(path, { state: "unread", alone: true });
        continue;
      }
      file = emptyFile(path, statusOf[kinds.get(path) ?? "modified"]);
    }
    if (file.path !== path) file = { ...file, path };
    const shown = before.file;
    const truncated = Boolean(result.truncated) && file === last;
    diffs.set(path, {
      state: "read",
      file: shown && sameFile(shown, file) ? shown : file,
      truncated,
      stale: false,
    });
  }
  return { ...state, diffs };
}

/** Reading `paths` failed: say so in their place, and let a reload try again. */
export function withDiffError(state: ChangesState, paths: readonly string[], error: string): ChangesState {
  const diffs = new Map(state.diffs);
  for (const path of paths) {
    if (diffs.get(path)?.state === "reading") diffs.set(path, { state: "failed", error });
  }
  return { ...state, diffs };
}

/** Lines added and removed across all the changed files, as the summary counts them. */
export function totals(changes: EnvironmentChanges | null): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const file of changes?.files ?? []) {
    additions += file.additions ?? 0;
    deletions += file.deletions ?? 0;
  }
  return { additions, deletions };
}

/** What a reader needs of the daemon. */
export type ChangesApi = {
  changes(threadId: string): Promise<EnvironmentChanges>;
  diff(params: ThreadDiffParams): Promise<ThreadDiffResult>;
};

/**
 * The changes of one thread, shared by every view of it (the tab and the side
 * panel read once between them) and kept while the user looks elsewhere.
 */
export class ThreadChangesReader {
  private state: ChangesState = EMPTY;
  private listeners = new Set<() => void>();
  /** A read of the list in flight, and whether another is due once it ends. */
  private listing: Promise<void> | null = null;
  private again = false;
  private lastKey: string | null = null;
  /** When the list was last asked for. */
  private askedAt = -Infinity;
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Diffs being read, so a view asking again in the meantime does not double them. */
  private inFlight = 0;

  constructor(
    readonly threadId: string,
    private readonly api: ChangesApi,
    private readonly errorText: (error: unknown) => string = String,
  ) {}

  getState = (): ChangesState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private set(state: ChangesState) {
    if (state === this.state) return;
    this.state = state;
    for (const listener of this.listeners) listener();
  }

  /** Read the list again after `delay` ms; a read already waiting is put off to then. */
  reload(delay = 0) {
    this.askedAt = Date.now();
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      this.list();
    }, delay);
  }

  /**
   * Read the list again after `delay` ms when `key` (what the agent has done so
   * far) moves on. Views that saw the same event pass the same key and only the
   * first asks; the first key only sets where to start from.
   */
  follow(key: string, delay: number) {
    const known = this.lastKey;
    this.lastKey = key;
    if (known !== null && known !== key) this.reload(delay);
  }

  /** Read the list unless it was asked for within `ms`: a view opening reads afresh. */
  refresh(ms: number) {
    if (Date.now() - this.askedAt >= ms) this.reload(0);
  }

  private list() {
    if (this.listing) {
      this.again = true;
      return;
    }
    this.set({ ...this.state, loading: true });
    this.listing = this.api
      .changes(this.threadId)
      .then(
        (changes) => this.set(withChanges(this.state, changes)),
        (error: unknown) => this.set({ ...this.state, loading: false, error: this.errorText(error) }),
      )
      .finally(() => {
        this.listing = null;
        if (this.again) {
          this.again = false;
          this.list();
        }
      });
  }

  /**
   * The files a view shows open, in order, the ones in view first: read the next
   * batch of those that need it. One batch at a time, so scrolling past a hundred
   * files does not queue a hundred reads; the view asks again when it lands.
   */
  want(paths: readonly string[]) {
    if (this.inFlight > 0 || !this.state.changes) return;
    const batch = nextBatch(this.state, paths);
    if (batch.length === 0) return;
    this.inFlight++;
    this.set(reading(this.state, batch));
    this.api
      .diff({ thread_id: this.threadId, paths: batch })
      .then(
        (result) => this.set(withDiff(this.state, batch, result)),
        (error: unknown) => this.set(withDiffError(this.state, batch, this.errorText(error))),
      )
      .finally(() => {
        this.inFlight--;
        // The view's wish is kept by its effect; tell it the reader is free.
        for (const listener of this.listeners) listener();
      });
  }

  /** Whether a batch of diffs is being read. */
  get busy(): boolean {
    return this.inFlight > 0;
  }

  dispose() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.listeners.clear();
  }
}

/**
 * A reader per thread while a view holds it, and the readers of the threads looked
 * at last after that, so going back to one shows it at once.
 */
export class ChangesReaders {
  private readers = new Map<string, { reader: ThreadChangesReader; holders: number }>();

  constructor(
    private readonly make: (threadId: string) => ThreadChangesReader,
    private readonly keep = 4,
  ) {}

  /** The reader of `threadId`, made if there is none, without holding it. */
  get(threadId: string): ThreadChangesReader {
    return this.entry(threadId).reader;
  }

  private entry(threadId: string) {
    let entry = this.readers.get(threadId);
    if (entry) this.readers.delete(threadId);
    else entry = { reader: this.make(threadId), holders: 0 };
    // Most recent last.
    this.readers.set(threadId, entry);
    this.evict();
    return entry;
  }

  /** The reader of `threadId`, and a function to let it go. */
  hold(threadId: string): [ThreadChangesReader, () => void] {
    const entry = this.entry(threadId);
    entry.holders++;
    let held = true;
    const release = () => {
      if (!held) return;
      held = false;
      entry.holders--;
      this.evict();
    };
    return [entry.reader, release];
  }

  private evict() {
    // The most recent stays, held or not: it was just asked for.
    const idle = [...this.readers].slice(0, -1).filter(([, entry]) => entry.holders === 0);
    for (const [threadId, entry] of idle.slice(0, Math.max(0, idle.length - this.keep))) {
      this.readers.delete(threadId);
      entry.reader.dispose();
    }
  }
}

const readCache = new WeakMap<ReadonlyMap<string, FileDiff>, DiffFile[]>();

/** The diffs read so far (or being read again), the same array while they are the same. */
export function readFiles(state: ChangesState): DiffFile[] {
  let files = readCache.get(state.diffs);
  if (!files) {
    files = [];
    for (const diff of state.diffs.values()) {
      if ((diff.state === "read" || diff.state === "reading") && diff.file) files.push(diff.file);
    }
    readCache.set(state.diffs, files);
  }
  return files;
}
