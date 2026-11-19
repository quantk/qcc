import { describe, expect, it } from "vitest";

import type { Thread } from "../api/protocol.gen";
import { hasWorkdir, liveDescendants, stayedAfter, treeWarning } from "./archive";

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
