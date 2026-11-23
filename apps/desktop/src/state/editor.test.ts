import { describe, expect, it } from "vitest";

import type { Thread } from "../api/protocol.gen";
import { editorTarget, loadEditor, saveEditor } from "./editor";

const thread = (patch: Partial<Thread>) => ({ id: "t", cwd: "/work/repo", ...patch }) as Thread;
const environment = (state: NonNullable<Thread["environment"]>["state"]) =>
  ({ id: "git-worktree", source_dir: "/work/repo", state }) as Thread["environment"];

describe("editorTarget", () => {
  it("opens the directory the thread works in", () => {
    expect(editorTarget(thread({}), undefined)).toEqual({ dir: "/work/repo" });
    expect(editorTarget(thread({ cwd: "/wt/task", environment: environment("ready") }), true)).toEqual({ dir: "/wt/task" });
  });

  it("offers nothing while the environment has no directory", () => {
    for (const state of ["pending", "failed", "removed"] as const) {
      expect(editorTarget(thread({ environment: environment(state) }), true)).toHaveProperty("reason");
    }
  });

  it("says when the directory is gone", () => {
    expect(editorTarget(thread({}), false)).toEqual({ reason: "/work/repo does not exist any more" });
  });
});

describe("the editor command", () => {
  it("is kept trimmed, and an empty one is forgotten", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => void values.set(key, value),
      removeItem: (key: string) => void values.delete(key),
    };
    expect(loadEditor(storage)).toBe("");
    saveEditor(storage, "  zed -n ");
    expect(loadEditor(storage)).toBe("zed -n");
    saveEditor(storage, " ");
    expect(values.size).toBe(0);
    expect(loadEditor(undefined)).toBe("");
  });
});
