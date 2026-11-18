import { describe, expect, it } from "vitest";

import type { Project } from "../api/protocol.gen";
import { newThreadStart, readNewThreadMemory, rememberNewThread } from "./newThread";

const app: Project = { id: "prj_app", name: "App", folders: ["/src/app", "/src/app-docs"], created_at: 0 };
const site: Project = { id: "prj_site", name: "Site", folders: ["/src/site"], created_at: 0 };
const projects = [app, site];

const started = { provider: "claude-code", mode: "", model: null, effort: null };

describe("newThreadStart", () => {
  it("starts outside any project when not opened from one", () => {
    const memory = { cwd: "/tmp/scratch", folders: { prj_app: "/src/app-docs" } };
    expect(newThreadStart(memory, undefined, projects)).toEqual({ project: null, cwd: "/tmp/scratch" });
    expect(newThreadStart({}, undefined, projects)).toEqual({ project: null, cwd: "" });
  });

  it("starts in the project it was opened from, in the folder last used there", () => {
    const memory = { cwd: "/tmp/scratch", folders: { prj_app: "/src/app-docs" } };
    expect(newThreadStart(memory, "prj_app", projects)).toEqual({ project: "prj_app", cwd: "/src/app-docs" });
    expect(newThreadStart(memory, "prj_site", projects)).toEqual({ project: "prj_site", cwd: "/src/site" });
  });

  it("falls back to the first folder when the remembered one left the project", () => {
    const memory = { folders: { prj_app: "/src/gone" } };
    expect(newThreadStart(memory, "prj_app", projects)).toEqual({ project: "prj_app", cwd: "/src/app" });
  });

  it("ignores a project that no longer exists", () => {
    expect(newThreadStart({ cwd: "/tmp" }, "prj_gone", projects)).toEqual({ project: null, cwd: "/tmp" });
  });
});

describe("rememberNewThread", () => {
  it("remembers the directory of a thread outside projects", () => {
    const next = rememberNewThread({ folders: { prj_app: "/src/app" } }, { ...started, project: undefined, cwd: "/tmp/x" });
    expect(next).toEqual({ provider: "claude-code", mode: "", folders: { prj_app: "/src/app" }, cwd: "/tmp/x" });
  });

  it("remembers a project folder for that project only", () => {
    const next = rememberNewThread({ cwd: "/tmp/x" }, { ...started, model: "haiku", project: app, cwd: "/src/app-docs" });
    expect(next.cwd).toBe("/tmp/x");
    expect(next.folders).toEqual({ prj_app: "/src/app-docs" });
    expect(next.model).toBe("haiku");
  });

  it("does not remember another directory picked inside a project", () => {
    const next = rememberNewThread({ cwd: "/tmp/x" }, { ...started, project: app, cwd: "/elsewhere" });
    expect(next.cwd).toBe("/tmp/x");
    expect(next.folders).toBeUndefined();
  });
});

describe("readNewThreadMemory", () => {
  it("reads nothing from garbage", () => {
    expect(readNewThreadMemory(null)).toEqual({});
    expect(readNewThreadMemory("x")).toEqual({});
  });

  it("turns the old project and directory into that project's folder", () => {
    const memory = readNewThreadMemory({ provider: "codex", cwd: "/src/app", project: "prj_app", model: "o3" });
    expect(memory.cwd).toBeUndefined();
    expect(memory.folders).toEqual({ prj_app: "/src/app" });
    expect(memory.provider).toBe("codex");
    expect(newThreadStart(memory, undefined, projects)).toEqual({ project: null, cwd: "" });
  });

  it("keeps an old directory chosen without a project", () => {
    expect(readNewThreadMemory({ cwd: "/tmp/x" }).cwd).toBe("/tmp/x");
  });

  it("round-trips what it remembers", () => {
    const memory = { provider: "codex", mode: "plan", cwd: "/tmp/x", folders: { prj_app: "/src/app" } };
    expect(readNewThreadMemory(JSON.parse(JSON.stringify(memory)))).toEqual({ ...memory, model: undefined, effort: undefined });
  });
});
