// MCP servers: the ones set up in qcc (a part of the daemon's settings) and how
// the agents report them doing, thread by thread.

import type { McpServerSettings, McpServerStatus, McpState, McpTransport, Settings, Thread } from "../api/protocol.gen";
import { type EnvRow, envOf, envRows } from "./settings";

export const mcpStateLabel: Record<McpState, string> = {
  starting: "Connecting",
  ready: "Ready",
  failed: "Failed",
  "needs-auth": "Needs sign-in",
  disabled: "Off",
};

/** The servers set up in qcc, by name. */
export function mcpServersOf(settings: Settings | undefined): [string, McpServerSettings][] {
  return Object.entries(settings?.mcp_servers ?? {}).sort(([a], [b]) => a.localeCompare(b));
}

/** Add, replace (`server`) or remove (`null`) one server. */
export function withMcpServer(settings: Settings, name: string, server: McpServerSettings | null): Settings {
  const servers = { ...settings.mcp_servers };
  if (server) servers[name] = server;
  else delete servers[name];
  return { ...settings, mcp_servers: servers };
}

/** Save a server under a new name, dropping the old one. */
export function renamedMcpServer(settings: Settings, from: string, to: string, server: McpServerSettings): Settings {
  return withMcpServer(from === to ? settings : withMcpServer(settings, from, null), to, server);
}

/** Why a name cannot be used, as the daemon would refuse it; null when it can. */
export function mcpNameProblem(name: string, taken: readonly string[]): string | null {
  if (!name) return "Give the server a name";
  if (!/^[A-Za-z0-9_-]+$/.test(name)) return "Letters, digits, _ and - only: agents put the name in tool names";
  if (taken.includes(name)) return `There already is a server called ${name}`;
  return null;
}

/** A server as one line: its command line or its URL. */
export function transportSummary(transport: McpTransport): string {
  return transport.type === "stdio" ? joinArgs([transport.command, ...(transport.args ?? [])]) : transport.url;
}

/** What the server dialog edits. */
export type McpDraft = {
  name: string;
  kind: "stdio" | "http";
  /** The program and its arguments, as typed in a shell. */
  commandLine: string;
  env: EnvRow[];
  url: string;
  headers: EnvRow[];
  disabled: boolean;
};

export function draftOf(name: string, server: McpServerSettings | undefined): McpDraft {
  const transport = server?.transport;
  return {
    name,
    kind: transport?.type ?? "stdio",
    commandLine: transport?.type === "stdio" ? transportSummary(transport) : "",
    env: transport?.type === "stdio" ? envRows(transport.env) : [],
    url: transport?.type === "http" ? transport.url : "",
    headers: transport?.type === "http" ? envRows(transport.headers) : [],
    disabled: server?.disabled ?? false,
  };
}

/** The server a draft describes, or why it does not describe one yet. */
export function serverOf(draft: McpDraft): McpServerSettings | { problem: string } {
  const disabled = draft.disabled || undefined;
  if (draft.kind === "http") {
    const url = draft.url.trim();
    if (!/^https?:\/\/./.test(url)) return { problem: "The URL must start with http:// or https://" };
    const headers = envOf(draft.headers);
    return { transport: { type: "http", url, ...(Object.keys(headers).length ? { headers } : {}) }, disabled };
  }
  const words = splitArgs(draft.commandLine);
  if (words === null) return { problem: "A quote is not closed" };
  const [command, ...args] = words;
  if (!command) return { problem: "Give the program that runs the server" };
  const env = envOf(draft.env);
  return {
    transport: {
      type: "stdio",
      command,
      ...(args.length ? { args } : {}),
      ...(Object.keys(env).length ? { env } : {}),
    },
    disabled,
  };
}

/**
 * Split a command line into words the way a shell would for plain quoting: spaces
 * separate, single and double quotes group, a backslash escapes the next character
 * outside single quotes. Null when a quote is left open.
 */
export function splitArgs(line: string): string[] | null {
  const words: string[] = [];
  let word = "";
  let inWord = false;
  let quote: '"' | "'" | null = null;
  for (let index = 0; index < line.length; index++) {
    const char = line.charAt(index);
    if (quote === "'") {
      if (char === "'") quote = null;
      else word += char;
    } else if (char === "\\" && index + 1 < line.length) {
      word += line.charAt(++index);
      inWord = true;
    } else if (quote === '"') {
      if (char === '"') quote = null;
      else word += char;
    } else if (char === '"' || char === "'") {
      quote = char;
      inWord = true;
    } else if (/\s/.test(char)) {
      if (inWord) words.push(word);
      word = "";
      inWord = false;
    } else {
      word += char;
      inWord = true;
    }
  }
  if (quote) return null;
  if (inWord) words.push(word);
  return words;
}

/** The inverse of `splitArgs`: words quoted only where they need it. */
export function joinArgs(words: readonly string[]): string {
  return words.map((word) => (word && /^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replaceAll("'", `'\\''`)}'`)).join(" ");
}

/** How one server is doing in the threads that have it. */
export type McpUsage = {
  /** Threads reporting it, by what they report. */
  counts: Partial<Record<McpState, number>>;
  /** The distinct reasons it failed. */
  errors: string[];
};

export function mcpUsage(threads: readonly Thread[], name: string): McpUsage {
  const counts: Partial<Record<McpState, number>> = {};
  const errors = new Set<string>();
  for (const thread of threads) {
    if (thread.archived) continue;
    const server = thread.mcp_servers?.find((candidate) => candidate.name === name);
    if (!server) continue;
    counts[server.state] = (counts[server.state] ?? 0) + 1;
    if (server.error) errors.add(server.error);
  }
  return { counts, errors: [...errors] };
}

/** A server agents report that qcc did not give them: from an agent's own configuration. */
export type AgentMcpServer = { name: string; providers: string[]; usage: McpUsage };

export function agentOwnMcpServers(threads: readonly Thread[], settings: Settings | undefined): AgentMcpServer[] {
  const ours = new Set(Object.keys(settings?.mcp_servers ?? {}));
  const providers = new Map<string, Set<string>>();
  for (const thread of threads) {
    if (thread.archived) continue;
    for (const server of thread.mcp_servers ?? []) {
      if (ours.has(server.name)) continue;
      const seen = providers.get(server.name) ?? new Set<string>();
      seen.add(thread.provider);
      providers.set(server.name, seen);
    }
  }
  return [...providers.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, seen]) => ({ name, providers: [...seen].sort(), usage: mcpUsage(threads, name) }));
}

/** One server on a thread's MCP tab. */
export type ThreadMcpRow = {
  name: string;
  /** What the session reports; absent for a server it was started without. */
  status: McpServerStatus | undefined;
  /** The thread keeps it off. */
  off: boolean;
  /** Set up in qcc rather than in the agent's own configuration. */
  ours: boolean;
  /** The thread keeps it off but the session still runs it: it has not taken the change yet. */
  pending: boolean;
  /** The session runs without it although the thread wants it: on from a session that takes the change. */
  pendingOn: boolean;
};

/**
 * The servers the session reports, then those the thread keeps off that it does not,
 * then qcc's that it does not: every server the thread can switch stays in sight.
 */
export function threadMcpRows(thread: Thread, settings: Settings | undefined): ThreadMcpRow[] {
  const ours = settings?.mcp_servers ?? {};
  const off = new Set(thread.mcp_disabled ?? []);
  const reported = thread.mcp_servers ?? [];
  // Only qcc's servers are the thread's to switch, so only theirs can wait for a
  // session to take a change; an agent's own may be off in its own settings.
  const rows: ThreadMcpRow[] = reported.map((status) => ({
    name: status.name,
    status,
    off: off.has(status.name),
    ours: status.name in ours,
    pending: status.name in ours && off.has(status.name) && status.state !== "disabled",
    pendingOn: status.name in ours && !off.has(status.name) && status.state === "disabled",
  }));
  const listed = (name: string) => rows.some((row) => row.name === name);
  for (const name of [...off].sort()) {
    if (!listed(name)) rows.push({ name, status: undefined, off: true, ours: name in ours, pending: false, pendingOn: false });
  }
  for (const [name, server] of mcpServersOf(settings)) {
    if (!server.disabled && !listed(name)) {
      rows.push({ name, status: undefined, off: false, ours: true, pending: false, pendingOn: false });
    }
  }
  return rows;
}
