import { describe, expect, it } from "vitest";

import type { PluginInfo, Project, Thread } from "../api/protocol.gen";
import {
  allowedModes,
  cardTime,
  childrenOf,
  dropOf,
  failureText,
  providerName,
  rootOf,
  threadGroups,
  threadTitle,
  threadTree,
  visiblePlugins,
} from "./selectors";

function thread(id: string, created_at: number, parent?: string): Thread {
  return {
    id,
    provider: "echo",
    cwd: "/work",
    parent: parent ?? null,
    status: "idle",
    created_at,
    updated_at: created_at,
    last_seq: 1,
  } as Thread;
}

describe("thread families", () => {
  const threads = [
    thread("a", 1),
    thread("b", 5),
    thread("a1", 2, "a"),
    thread("a2", 3, "a"),
    thread("a1x", 4, "a1"),
    thread("orphan", 6, "gone"),
  ];

  it("puts children under their parent, in the order they were started", () => {
    const rows = threadTree(threads).map((row) => `${"-".repeat(row.depth)}${row.thread.id}:${row.children}`);
    expect(rows).toEqual(["orphan:0", "b:0", "a:2", "-a1:1", "--a1x:0", "-a2:0"]);
  });

  it("hides the family of a collapsed thread, and tells what the hidden ones need", () => {
    const asking = { ...thread("a1x", 4, "a1"), pending_interactions: [{}] } as unknown as Thread;
    const family = threads.map((t) => (t.id === "a1x" ? asking : t));
    const rows = threadTree(family, new Set(["a"]));
    expect(rows.map((row) => row.thread.id)).toEqual(["orphan", "b", "a"]);
    const a = rows[2];
    expect([a?.collapsed, a?.hidden, a?.hiddenAttention]).toEqual([true, 3, "waiting"]);
    // A thread without children has nothing to collapse.
    expect(threadTree(family, new Set(["b"]))[1]?.collapsed).toBe(false);
  });

  it("never hides the selected thread", () => {
    const rows = threadTree(threads, new Set(["a", "a1"]), "a1x").map((row) => row.thread.id);
    expect(rows).toEqual(["orphan", "b", "a", "a1", "a1x", "a2"]);
  });

  it("finds the root of a family and the children of a thread", () => {
    const byId: Record<string, Thread> = Object.fromEntries(threads.map((t) => [t.id, t]));
    const find = (id: string) => byId[id] as Thread;
    expect(rootOf(find("a1x"), byId).id).toBe("a");
    expect(rootOf(find("orphan"), byId).id).toBe("orphan");
    expect(childrenOf(threads, "a").map((t) => t.id)).toEqual(["a1", "a2"]);
  });
});

describe("example plugins", () => {
  const plugin = (id: string, example: boolean) =>
    ({ id, example, version: "1", bundled: true, providers: [], environments: [], commands: [], skills: [] }) as unknown as PluginInfo;

  it("are left out unless asked for", () => {
    const plugins = [plugin("real", false), plugin("echo", true)];
    expect(visiblePlugins(plugins, false).map((p) => p.id)).toEqual(["real"]);
    expect(visiblePlugins(plugins, true).map((p) => p.id)).toEqual(["real", "echo"]);
  });
});

describe("sidebar groups", () => {
  const project = (id: string, created_at: number): Project => ({ id, name: id, folders: [], created_at });
  const at = (id: string, updated_at: number, extra: Partial<Thread>) => ({ ...thread(id, 1), updated_at, ...extra }) as Thread;

  it("puts projects first, empty ones too, and the rest by folder", () => {
    const threads = [
      at("p1", 10, { project: "shop" }),
      at("child", 50, { parent: "p1", cwd: "/elsewhere" }),
      at("loose", 30, { cwd: "/tmp/repo" }),
      at("lost", 40, { project: "deleted", cwd: "/old" }),
    ];
    const all = Object.fromEntries(threads.map((t) => [t.id, t]));
    const groups = threadGroups(threads, all, [project("shop", 1), project("empty", 20)]);
    expect(groups.map((g) => [g.key, g.threads.map((t) => t.id)])).toEqual([
      ["project:shop", ["p1", "child"]],
      ["project:empty", []],
      ["folder:/old", ["lost"]],
      ["folder:/tmp/repo", ["loose"]],
    ]);
  });
});

describe("permission modes of a child", () => {
  const all = ["ask", "accept-edits", "auto", "full"] as const;
  const parent = (permission_mode?: string) => ({ ...thread("p", 1), permission_mode }) as unknown as Thread;

  it("go no looser than the parent's", () => {
    expect(allowedModes(all, undefined)).toEqual({ modes: [...all], agentDefault: true });
    expect(allowedModes(all, parent("accept-edits"))).toEqual({ modes: ["ask", "accept-edits"], agentDefault: false });
    expect(allowedModes(all, parent())).toEqual({ modes: ["ask"], agentDefault: true });
  });
});

describe("dragging a thread in the sidebar", () => {
  const mode = (t: Thread, permission_mode: string) => ({ ...t, permission_mode }) as unknown as Thread;
  const group = [
    mode(thread("a", 1), "full"),
    mode(thread("a1", 2, "a"), "full"),
    mode(thread("b", 3), "ask"),
    thread("c", 4),
    thread("x", 5, "elsewhere"),
  ];

  it("puts it under a thread, next to one, or on its own", () => {
    expect(dropOf(group, "b", "a", "into")).toEqual({ parent: "a" });
    expect(dropOf(group, "b", "a1", "beside")).toEqual({ parent: "a" });
    expect(dropOf(group, "a1", "c", "beside")).toEqual({ parent: null });
    expect(dropOf(group, "a1", null, "into")).toEqual({ parent: null });
    // Next to a thread whose parent is in another group is the top of this one.
    expect(dropOf(group, "a1", "x", "beside")).toEqual({ parent: null });
  });

  it("does nothing where it already is", () => {
    expect(dropOf(group, "a1", "a", "into")).toBeNull();
    expect(dropOf(group, "b", null, "into")).toBeNull();
    expect(dropOf(group, "b", "missing", "into")).toBeNull();
  });

  it("refuses what the daemon would", () => {
    expect(dropOf(group, "a", "a1", "into")).toHaveProperty("refused");
    expect(dropOf(group, "a", "a", "into")).toHaveProperty("refused");
    expect(dropOf(group, "a", "b", "into")).toHaveProperty("refused");
    // Without a mode it may sit only under a thread without one.
    expect(dropOf(group, "c", "b", "into")).toHaveProperty("refused");
    expect(dropOf(group, "b", "c", "into")).toEqual({ parent: "c" });
  });
});

describe("what cards and rows say about a thread", () => {
  const plugins = [{ id: "p", providers: [{ id: "echo", name: "Echo" }] }] as unknown as PluginInfo[];

  it("names an agent by its plugin, or by its id when no plugin provides it", () => {
    expect(providerName(plugins, "echo")).toBe("Echo");
    expect(providerName(plugins, "gone")).toBe("gone");
  });

  it("calls a thread without a title a new thread", () => {
    expect(threadTitle({ ...thread("a", 1), title: "Fix the build" })).toBe("Fix the build");
    expect(threadTitle(thread("a", 1))).toBe("New thread");
    expect(threadTitle(undefined)).toBe("New thread");
  });

  it("tells a failed session from a failed turn", () => {
    expect(failureText({ ...thread("a", 1), status: "failed" })).toBe("The session failed");
    expect(failureText({ ...thread("a", 1), last_turn_status: "failed" })).toBe("The last turn failed");
  });

  it("shows how long a turn runs while it does, and how long ago otherwise", () => {
    const now = 10 * 60_000;
    const working = { ...thread("a", 0), status: "active", turn_started_at: now - 65_000 } as Thread;
    expect(cardTime(working, now)).toBe("1m 05s");
    expect(cardTime({ ...thread("a", 0), updated_at: now - 5 * 60_000 }, now)).toBe("5m");
  });
});
