import { describe, expect, it } from "vitest";

import type { Api, Notification } from "../api/api";
import type { ArchiveProgress, ArchiveStep, Thread } from "../api/protocol.gen";
import {
  archiveSummary,
  hasWorkdir,
  isFinal,
  liveDescendants,
  stayedAfter,
  stepLabel,
  treeOrder,
  treeWarning,
} from "./archive";
import { AppStore } from "./store";

function thread(id: string, created_at: number, parent: string | null, more: Partial<Thread> = {}): Thread {
  return { id, provider: "echo", cwd: "/work", parent, status: "idle", created_at, updated_at: 0, last_seq: 1, title: id, ...more } as Thread;
}

const ready = { id: "git-worktree", source_dir: "/work", state: "ready" } as Thread["environment"];

// p started b (working) and a (archived, which started a1); a1 started a11; h is a
// plugin's hidden thread under p; x belongs elsewhere.
const root = thread("p", 0, null);
const a1 = thread("a1", 3, "a", { environment: ready });
const family = [
  root,
  thread("b", 2, "p", { status: "active" }),
  thread("a", 1, "p", { archived: true }),
  a1,
  thread("a11", 4, "a1"),
  thread("h", 5, "p", { hidden: true }),
  thread("x", 6, "other"),
];

describe("liveDescendants", () => {
  it("lists what is not archived yet at any depth, looking through archived threads", () => {
    const members = liveDescendants(family, "p").map((m) => [m.thread.id, m.depth]);
    expect(members).toEqual([
      ["a1", 2],
      ["a11", 3],
      ["b", 1],
      ["h", 1],
    ]);
  });

  it("is empty for a thread that started nothing", () => {
    expect(liveDescendants(family, "x")).toEqual([]);
  });
});

describe("treeWarning", () => {
  const members = liveDescendants(family, "p");

  it("waits for the environments before it can be confirmed", () => {
    const warning = treeWarning(root, members, null);
    expect(warning).toMatchObject({ checking: true, working: 1, uncommitted: 0, force: false });
    expect(warning.title).toBe('Archive "p" and 4 threads under it?');
  });

  it("archives without force when no uncommitted work was found", () => {
    expect(treeWarning(root, members, new Set())).toMatchObject({
      checking: false,
      force: false,
      confirm: "Archive 5 threads",
    });
  });

  it("forces only once the user is told whose work is discarded, the root's included", () => {
    const warning = treeWarning(root, members, new Set(["a1", "p", "x"]));
    expect(warning).toMatchObject({ uncommitted: 2, force: true, confirm: "Archive 5 and discard changes" });
  });

  it("says thread for one", () => {
    expect(treeWarning(root, members.slice(0, 1), new Set()).title).toBe('Archive "p" and 1 thread under it?');
  });
});

describe("hasWorkdir", () => {
  it("only a ready environment has a directory to lose work in", () => {
    expect(hasWorkdir(a1)).toBe(true);
    expect(hasWorkdir(thread("y", 0, null, { environment: { ...ready!, state: "removed" } }))).toBe(false);
    expect(hasWorkdir(root)).toBe(false);
  });
});

describe("stayedAfter", () => {
  const byId = Object.fromEntries(family.map((t) => [t.id, t]));

  it("is null when everything was archived", () => {
    expect(stayedAfter({}, byId)).toBeNull();
    expect(stayedAfter({ refused: [], kept: [] }, byId)).toBeNull();
  });

  it("names the refused threads with their reason, then those kept above them", () => {
    const result = { refused: [{ thread_id: "a1", code: -32006, message: "has 1 uncommitted change" }], kept: ["p", "gone"] };
    expect(stayedAfter(result, byId)).toEqual([
      { id: "a1", title: "a1", reason: "has 1 uncommitted change" },
      { id: "p", title: "p", reason: "a thread under it stayed" },
      { id: "gone", title: "gone", reason: "a thread under it stayed" },
    ]);
  });
});

function step(thread_id: string, parent: string | null, step: ArchiveStep): ArchiveProgress {
  return { thread_id, title: `title ${thread_id}`, parent, step };
}

// The daemon's order: deepest first, the thread asked for last.
const report = [
  step("a11", "a1", { kind: "archived" }),
  step("a1", "p", { kind: "removing_environment", environment: "Git worktree" }),
  step("b", "p", { kind: "waiting" }),
  step("p", null, { kind: "waiting" }),
];

describe("archive progress", () => {
  it("says what each step is doing", () => {
    expect(stepLabel({ kind: "stopping_session", working: true })).toBe("Stopping the agent");
    expect(stepLabel({ kind: "stopping_session", working: false })).toBe("Closing the session");
    expect(stepLabel({ kind: "removing_environment", environment: "Git worktree" })).toBe("Removing its Git worktree");
    expect(stepLabel({ kind: "refused", code: -32006, message: "1 uncommitted change" })).toBe("1 uncommitted change");
    expect(isFinal({ kind: "kept" })).toBe(true);
    expect(isFinal({ kind: "waiting" })).toBe(false);
  });

  it("sums up how far a tree got, counting the thread under way as half", () => {
    const summary = archiveSummary(report);
    expect(summary).toMatchObject({ total: 4, done: 1, percent: 38, headline: "Archiving 4 threads" });
    expect(summary.current?.thread_id).toBe("a1");
    const one = archiveSummary([step("p", null, { kind: "archived" })]);
    expect(one).toMatchObject({ total: 1, done: 1, percent: 100, current: null, headline: 'Archiving "title p"' });
  });

  it("shows the tree parents first, though it is archived deepest first", () => {
    // Siblings in the order they were started, when that is known.
    const started: Record<string, number> = { b: 1, a1: 2 };
    expect(treeOrder(report, (id) => started[id]).map(({ thread, depth }) => [thread.thread_id, depth])).toEqual([
      ["p", 0],
      ["b", 1],
      ["a1", 1],
      ["a11", 2],
    ]);
    // Otherwise as the daemon listed them.
    expect(treeOrder(report).map(({ thread }) => thread.thread_id)).toEqual(["p", "a1", "a11", "b"]);
    // A thread under an archived one stands under the thread asked for.
    const looked = [step("x", "gone", { kind: "waiting" }), step("p", null, { kind: "waiting" })];
    expect(treeOrder(looked).map(({ thread, depth }) => [thread.thread_id, depth])).toEqual([
      ["p", 0],
      ["x", 1],
    ]);
  });

  it("follows the reports of the archive the store asked for, and forgets it once answered", async () => {
    let notify: (notification: Notification) => void = () => {};
    let answer: (result: unknown) => void = () => {};
    const calls: { method: string; params: Record<string, unknown> }[] = [];
    const api: Api = {
      call: ((method: string, params: Record<string, unknown>) => {
        calls.push({ method, params });
        return new Promise((resolve) => (answer = resolve as (result: unknown) => void));
      }) as Api["call"],
      onNotification: (handler) => {
        notify = handler;
        return () => {};
      },
      onDaemonState: () => () => {},
      daemonState: () => new Promise(() => {}),
    };
    const app = new AppStore(api, (run) => run());
    app.start();
    const done = app.archive("p", false);
    expect(app.getState().archives).toEqual({ p: [] });
    const progress = calls[0]?.params.progress as number;
    expect(calls[0]).toMatchObject({ method: "thread.archive", params: { thread_id: "p", descendants: true } });

    notify({ method: "thread.archive_progress", params: { progress, threads: report } });
    // Another call's reports are not this one's.
    notify({ method: "thread.archive_progress", params: { progress: progress + 100, threads: [] } });
    expect(app.getState().archives.p).toEqual(report);

    answer({ thread: thread("p", 0, null, { archived: true }) });
    expect(await done).toBeNull();
    expect(app.getState().archives).toEqual({});
  });
});
