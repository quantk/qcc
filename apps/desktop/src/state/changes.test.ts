import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ChangeScope, ChangedFile, EnvironmentChanges, ThreadDiffParams, ThreadDiffResult } from "../api/protocol.gen";
import {
  BATCH,
  type ChangesState,
  ChangesReaders,
  EMPTY,
  FOLD_ABOVE,
  ThreadChangesReader,
  isGenerated,
  nextBatch,
  reading,
  sameFile,
  startsFolded,
  totals,
  withChanges,
  withDiff,
  withDiffError,
} from "./changes";
import { parseDiff } from "./diff";

function changed(path: string, fields: Partial<ChangedFile> = {}): ChangedFile {
  return { path, kind: "modified", additions: 1, deletions: 1, ...fields };
}

function list(...files: ChangedFile[]): EnvironmentChanges {
  return { files, branch: null, base: null };
}

function patch(path: string, before: string, after: string): string {
  return `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-${before}\n+${after}\n`;
}

const A = changed("a.ts");
const B = changed("b.ts");

describe("which files start folded", () => {
  it("folds lockfiles and build output whatever their size", () => {
    expect(["package-lock.json", "web/pnpm-lock.yaml", "Cargo.lock", "go.sum", "dist/app.min.js"].map(isGenerated)).toEqual([
      true,
      true,
      true,
      true,
      true,
    ]);
    expect(isGenerated("src/lock.ts")).toBe(false);
    expect(startsFolded(changed("yarn.lock", { additions: 1 }))).toBe(true);
  });

  it("folds files with many changed lines, a new file once its diff is read", () => {
    expect(startsFolded(changed("a.ts", { additions: FOLD_ABOVE, deletions: 1 }))).toBe(true);
    expect(startsFolded(changed("a.ts", { additions: 10 }))).toBe(false);
    const fresh = changed("new.ts", { kind: "untracked", additions: null, deletions: null });
    expect(startsFolded(fresh)).toBe(false);
    const long = { ...parseDiff(patch("new.ts", "a", "b"))[0]!, lines: new Array(FOLD_ABOVE + 1).fill({ kind: "add", text: "" }) };
    expect(startsFolded(fresh, long)).toBe(true);
  });
});

describe("reading diffs a few files at a time", () => {
  const state = withChanges(EMPTY, list(A, B, changed("img.png", { additions: null, deletions: null })));

  it("reads the files wanted that are not read, and nothing for binary files", () => {
    expect(nextBatch(state, ["img.png", "b.ts", "a.ts", "gone.ts"])).toEqual(["b.ts", "a.ts"]);
    const many = Array.from({ length: BATCH + 5 }, (_, index) => changed(`f${index}.ts`));
    const big = withChanges(EMPTY, list(...many));
    expect(nextBatch(big, many.map((file) => file.path))).toHaveLength(BATCH);
  });

  it("puts each file of the answer in its place", () => {
    const asked = reading(state, ["a.ts", "b.ts"]);
    expect(nextBatch(asked, ["a.ts", "b.ts"])).toEqual([]);
    const read = withDiff(asked, ["a.ts", "b.ts"], { diff: patch("a.ts", "x", "y") + patch("b.ts", "p", "q") });
    const a = read.diffs.get("a.ts");
    expect(a?.state === "read" && a.file.lines.map((line) => line.text)).toEqual(["@@ -1 +1 @@", "x", "y"]);
    expect(read.diffs.get("b.ts")?.state).toBe("read");
    expect(nextBatch(read, ["a.ts", "b.ts"])).toEqual([]);
  });

  it("takes a file without a textual change as empty", () => {
    const read = withDiff(reading(state, ["a.ts"]), ["a.ts"], { diff: "" });
    const a = read.diffs.get("a.ts");
    expect(a?.state === "read" && a.file.lines).toEqual([]);
  });

  it("reads a file missing from a shared answer again on its own", () => {
    const asked = reading(state, ["a.ts", "b.ts"]);
    const read = withDiff(asked, ["a.ts", "b.ts"], { diff: patch("a.ts", "x", "y"), truncated: true });
    const a = read.diffs.get("a.ts");
    expect(a?.state === "read" && a.truncated).toBe(true);
    expect(read.diffs.get("b.ts")).toEqual({ state: "unread", alone: true });
    expect(nextBatch(read, ["b.ts", "a.ts"])).toEqual(["b.ts"]);
    // Alone, whatever git calls the file, the answer is its diff.
    const alone = withDiff(reading(read, ["b.ts"]), ["b.ts"], { diff: patch('"b.ts"', "p", "q") });
    const b = alone.diffs.get("b.ts");
    expect(b?.state === "read" && b.file.path).toBe("b.ts");
  });

  it("says when reading failed, and tries again after a reload", () => {
    const failed = withDiffError(reading(state, ["a.ts"]), ["a.ts"], "boom");
    expect(failed.diffs.get("a.ts")).toEqual({ state: "failed", error: "boom" });
    expect(nextBatch(failed, ["a.ts"])).toEqual([]);
    expect(nextBatch(withChanges(failed, list(A, B)), ["a.ts"])).toEqual(["a.ts"]);
  });
});

describe("a new list of changes", () => {
  const read = withDiff(reading(withChanges(EMPTY, list(A, B)), ["a.ts", "b.ts"]), ["a.ts", "b.ts"], {
    diff: patch("a.ts", "x", "y") + patch("b.ts", "p", "q"),
  });

  it("keeps what is shown, to read again, and drops files no longer changed", () => {
    const next = withChanges(read, list(A));
    expect([...next.diffs.keys()]).toEqual(["a.ts"]);
    const a = next.diffs.get("a.ts");
    expect(a?.state === "read" && a.stale).toBe(true);
    expect(nextBatch(next, ["a.ts"])).toEqual(["a.ts"]);
  });

  it("keeps the very file shown when it reads the same, and takes a changed one", () => {
    const shown = (state: ChangesState) => {
      const a = state.diffs.get("a.ts");
      return a?.state === "read" ? a.file : null;
    };
    const again = reading(withChanges(read, list(A, B)), ["a.ts"]);
    const reread = again.diffs.get("a.ts");
    expect(reread?.state === "reading" && reread.file).toBe(shown(read));
    expect(shown(withDiff(again, ["a.ts"], { diff: patch("a.ts", "x", "y") }))).toBe(shown(read));
    const changedFile = shown(withDiff(again, ["a.ts"], { diff: patch("a.ts", "x", "z") }));
    expect(changedFile).not.toBe(shown(read));
    expect(changedFile?.lines.at(-1)?.text).toBe("z");
  });

  it("ignores an answer for files no longer asked for", () => {
    const asked = reading(read, ["b.ts"]);
    const dropped = withChanges(asked, list(A));
    expect(withDiff(dropped, ["b.ts"], { diff: patch("b.ts", "p", "r") }).diffs.has("b.ts")).toBe(false);
  });
});

describe("sameFile and totals", () => {
  it("compares the lines a file shows", () => {
    const [a] = parseDiff(patch("a.ts", "x", "y"));
    const [b] = parseDiff(patch("a.ts", "x", "y"));
    const [c] = parseDiff(patch("a.ts", "x", "Y"));
    expect(sameFile(a!, b!)).toBe(true);
    expect(sameFile(a!, c!)).toBe(false);
  });

  it("adds up the summary, new files counting nothing", () => {
    expect(totals(list(changed("a", { additions: 3, deletions: 1 }), changed("n", { kind: "untracked", additions: null, deletions: null })))).toEqual({
      additions: 3,
      deletions: 1,
    });
    expect(totals(null)).toEqual({ additions: 0, deletions: 0 });
  });
});

describe("ThreadChangesReader", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function fake() {
    const calls: { changes: number; diffs: ThreadDiffParams[] } = { changes: 0, diffs: [] };
    let files = list(A, B);
    const api = {
      changes: async () => {
        calls.changes++;
        return files;
      },
      diff: async (params: ThreadDiffParams): Promise<ThreadDiffResult> => {
        calls.diffs.push(params);
        return { diff: (params.paths ?? []).map((path) => patch(path, "x", "y")).join("") };
      },
    };
    return { calls, api, setFiles: (next: EnvironmentChanges) => (files = next) };
  }

  it("reads the list once for views that saw the same event", async () => {
    const { calls, api } = fake();
    const reader = new ThreadChangesReader("t", api);
    reader.follow("1", 800);
    reader.refresh(2000);
    reader.refresh(2000);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.changes).toBe(1);
    expect(reader.getState().changes?.files).toHaveLength(2);
    reader.follow("2", 800);
    reader.follow("2", 800);
    await vi.advanceTimersByTimeAsync(800);
    expect(calls.changes).toBe(2);
    reader.follow("2", 800);
    await vi.advanceTimersByTimeAsync(800);
    expect(calls.changes).toBe(2);
  });

  it("reads another scope afresh and drops what was read for the last one", async () => {
    const { calls, api } = fake();
    const scopes: unknown[] = [];
    const commits = { ...list(A, B), revisions: [{ id: "ccc3333", title: "Fix" }, { id: "bbb2222", title: "Add" }] };
    const reader = new ThreadChangesReader("t", {
      ...api,
      changes: async (_id: string, scope: ChangeScope) => {
        scopes.push(scope);
        return api.changes().then(() => commits);
      },
    });
    reader.reload();
    await vi.advanceTimersByTimeAsync(0);
    reader.want(["a.ts"]);
    // The scope changes while the diff is read: its answer belongs to the old one.
    reader.setScope({ kind: "revisions", from: "bbb2222", to: "ccc3333" });
    expect(reader.getState().changes).toBeNull();
    expect(reader.getState().revisions).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(0);
    expect(scopes.at(-1)).toEqual({ kind: "revisions", from: "bbb2222", to: "ccc3333" });
    expect(reader.getState().diffs.size).toBe(0);
    reader.want(["a.ts"]);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.diffs.at(-1)).toEqual({
      thread_id: "t",
      paths: ["a.ts"],
      scope: { kind: "revisions", from: "bbb2222", to: "ccc3333" },
    });

    // The commit it showed was amended away: back to all the work.
    commits.revisions = [{ id: "ddd4444", title: "Fix" }, { id: "bbb2222", title: "Add" }];
    reader.reload();
    // The read that finds the commit gone, then the read of all the work.
    await vi.runAllTimersAsync();
    expect(reader.getState().scope).toEqual({ kind: "all" });
    expect(scopes.at(-1)).toEqual({ kind: "all" });
    expect(reader.getState().changes?.files).toHaveLength(2);
  });

  it("reads one batch of diffs at a time, and reads stale ones again", async () => {
    const { calls, api } = fake();
    const reader = new ThreadChangesReader("t", api);
    const told = vi.fn();
    reader.subscribe(told);
    reader.reload();
    await vi.advanceTimersByTimeAsync(0);
    reader.want(["a.ts", "b.ts"]);
    reader.want(["a.ts", "b.ts"]);
    expect(reader.busy).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.diffs).toEqual([{ thread_id: "t", paths: ["a.ts", "b.ts"] }]);
    expect(reader.getState().diffs.get("a.ts")?.state).toBe("read");
    expect(told).toHaveBeenCalled();

    reader.want(["a.ts"]);
    expect(calls.diffs).toHaveLength(1);
    reader.reload();
    await vi.advanceTimersByTimeAsync(0);
    reader.want(["a.ts"]);
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.diffs.at(-1)).toEqual({ thread_id: "t", paths: ["a.ts"] });
  });

  it("keeps readers that views hold, and the last few others", () => {
    const made: string[] = [];
    const readers = new ChangesReaders((id) => {
      made.push(id);
      return new ThreadChangesReader(id, fake().api);
    }, 1);
    const [first, releaseFirst] = readers.hold("a");
    const [again, releaseAgain] = readers.hold("a");
    expect(again).toBe(first);
    releaseFirst();
    releaseAgain();
    readers.hold("b")[1]();
    const [, releaseC] = readers.hold("c");
    // "a" was let go first: besides the one just asked for, one idle reader is kept.
    readers.hold("b");
    readers.hold("a");
    releaseC();
    expect(made).toEqual(["a", "b", "c", "a"]);
  });
});
