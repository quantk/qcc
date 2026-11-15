import { describe, expect, it } from "vitest";

import type { McpServerStatus, Settings, Thread } from "../api/protocol.gen";
import {
  agentOwnMcpServers,
  draftOf,
  joinArgs,
  mcpNameProblem,
  mcpTrouble,
  mcpUsage,
  renamedMcpServer,
  serverOf,
  splitArgs,
  withMcpServer,
} from "./mcp";

function thread(id: string, provider: string, servers: McpServerStatus[], archived = false): Thread {
  return {
    id,
    provider,
    cwd: "/work",
    status: "idle",
    created_at: 1,
    updated_at: 1,
    last_seq: 1,
    mcp_servers: servers,
    archived,
  } as Thread;
}

describe("command lines", () => {
  it("split as a shell would and join back", () => {
    expect(splitArgs(`npx -y "my server" 'a "b"' c\\ d`)).toEqual(["npx", "-y", "my server", 'a "b"', "c d"]);
    expect(splitArgs(`  `)).toEqual([]);
    expect(splitArgs(`""`)).toEqual([""]);
    expect(splitArgs(`npx "open`)).toBeNull();
    const words = ["npx", "-y", "my server", "it's", "", "--url=https://x/y"];
    expect(splitArgs(joinArgs(words))).toEqual(words);
    expect(joinArgs(["npx", "-y", "docs"])).toBe("npx -y docs");
  });
});

describe("server drafts", () => {
  it("become the server they describe, and come back from it", () => {
    const draft = { ...draftOf("docs", undefined), commandLine: "npx -y docs", env: [{ name: "TOKEN", value: "t" }] };
    const server = serverOf(draft);
    expect(server).toEqual({
      transport: { type: "stdio", command: "npx", args: ["-y", "docs"], env: { TOKEN: "t" } },
      disabled: undefined,
    });
    expect(draftOf("docs", server as never)).toEqual({ ...draft, disabled: false });

    const http = serverOf({ ...draftOf("web", undefined), kind: "http", url: " https://x/mcp ", disabled: true });
    expect(http).toEqual({ transport: { type: "http", url: "https://x/mcp" }, disabled: true });
  });

  it("say what is missing", () => {
    expect(serverOf(draftOf("docs", undefined))).toEqual({ problem: "Give the program that runs the server" });
    expect(serverOf({ ...draftOf("docs", undefined), commandLine: "npx 'x" })).toEqual({ problem: "A quote is not closed" });
    expect(serverOf({ ...draftOf("web", undefined), kind: "http", url: "ftp://x" })).toHaveProperty("problem");
    expect(mcpNameProblem("docs-2_x", [])).toBeNull();
    expect(mcpNameProblem("my docs", [])).toMatch(/Letters/);
    expect(mcpNameProblem("docs", ["docs"])).toMatch(/already/);
  });
});

describe("settings", () => {
  it("add, rename and remove servers without touching the rest", () => {
    const server = { transport: { type: "stdio" as const, command: "npx" } };
    const base: Settings = { default_provider: "codex" };
    const added = withMcpServer(base, "docs", server);
    expect(added).toEqual({ default_provider: "codex", mcp_servers: { docs: server } });
    expect(renamedMcpServer(added, "docs", "notes", server).mcp_servers).toEqual({ notes: server });
    expect(withMcpServer(added, "docs", null).mcp_servers).toEqual({});
  });
});

describe("statuses", () => {
  const threads = [
    thread("a", "codex", [
      { name: "docs", state: "ready" },
      { name: "playwright", state: "failed", error: "no browser" },
    ]),
    thread("b", "claude-code", [
      { name: "docs", state: "failed", error: "ENOENT" },
      { name: "playwright", state: "ready" },
    ]),
    thread("c", "codex", [{ name: "docs", state: "failed", error: "ENOENT" }], true),
  ];

  it("are summed up over the live threads", () => {
    expect(mcpUsage(threads, "docs")).toEqual({ counts: { ready: 1, failed: 1 }, errors: ["ENOENT"] });
    expect(mcpUsage(threads, "nope")).toEqual({ counts: {}, errors: [] });
  });

  it("tell the agents' own servers from qcc's", () => {
    const settings: Settings = { mcp_servers: { docs: { transport: { type: "stdio", command: "npx" } } } };
    expect(agentOwnMcpServers(threads, settings)).toEqual([
      {
        name: "playwright",
        providers: ["claude-code", "codex"],
        usage: { counts: { failed: 1, ready: 1 }, errors: ["no browser"] },
      },
    ]);
  });

  it("pick out the servers that need the user", () => {
    expect(mcpTrouble(threads[0] as Thread).map((server) => server.name)).toEqual(["playwright"]);
    expect(mcpTrouble(thread("d", "codex", [{ name: "x", state: "needs-auth" }]))).toHaveLength(1);
  });
});
