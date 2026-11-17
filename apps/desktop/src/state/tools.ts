// What agents used — commands, tools, MCP tools, skills — how often, how the uses
// ended and how long they took. The same rows for one thread, counted live from
// its timeline, and for many, added up from the daemon's hourly buckets
// (`usage.tools`).

import type { Item, ToolBucket, ToolKind } from "../api/protocol.gen";
import type { Entry } from "./timeline";

export type ToolUse = { kind: ToolKind; name: string; server: string | null };

/**
 * What an item says was used. Mirrors the daemon's fold (`ToolUse::of`): the
 * bridge marks MCP tools and skills, so nothing here knows an agent's dialect.
 */
export function toolUseOf(item: Item): ToolUse | null {
  switch (item.type) {
    case "command":
      return { kind: "command", name: program(item.command), server: null };
    case "fileChange":
    case "fileRead":
    case "search":
      return { kind: item.type, name: "", server: null };
    case "tool":
      if (item.skill) return { kind: "skill", name: item.skill, server: null };
      if (item.mcp_server) return { kind: "mcp", name: item.name, server: item.mcp_server };
      return { kind: "tool", name: item.name, server: null };
    default:
      return null;
  }
}

/** The program a command line runs, as the daemon reads it. */
export function program(command: string): string {
  const word = command.split(/\s+/).find((word) => word && (!word.includes("=") || /^[-/.]/.test(word))) ?? "";
  return (word.split("/").at(-1) ?? word).slice(0, 60);
}

export type ToolRow = ToolUse & {
  key: string;
  calls: number;
  failed: number;
  declined: number;
  interrupted: number;
  durationMs: number;
  longestMs: number;
  /** Threads that used it. */
  threads: number;
};

const keyOf = (use: ToolUse) => `${use.kind}\u0000${use.server ?? ""}\u0000${use.name}`;

class Rows {
  private rows = new Map<string, ToolRow & { seen: Set<string> }>();

  row(use: ToolUse, threadId: string) {
    const key = keyOf(use);
    let row = this.rows.get(key);
    if (!row) {
      row = { ...use, key, calls: 0, failed: 0, declined: 0, interrupted: 0, durationMs: 0, longestMs: 0, threads: 0, seen: new Set() };
      this.rows.set(key, row);
    }
    row.seen.add(threadId);
    return row;
  }

  /** Most used first. */
  done(): ToolRow[] {
    return [...this.rows.values()]
      .map(({ seen, ...row }) => ({ ...row, threads: seen.size }))
      .sort((a, b) => b.calls - a.calls || a.name.localeCompare(b.name));
  }
}

/** One thread's uses, from its timeline. Items still running are not counted. */
export function toolsOfTimeline(entries: readonly Entry[]): ToolRow[] {
  const rows = new Rows();
  for (const entry of entries) {
    if (entry.kind !== "item" || entry.item.status === "pending") continue;
    const use = toolUseOf(entry.item);
    if (!use) continue;
    const row = rows.row(use, "");
    row.calls += 1;
    switch (entry.item.status) {
      case "failed":
        row.failed += 1;
        break;
      case "declined":
        row.declined += 1;
        break;
      case "interrupted":
        row.interrupted += 1;
        break;
      default:
        if (entry.item.type === "command" && entry.item.exit_code != null && entry.item.exit_code !== 0) row.failed += 1;
    }
    if (entry.endTs !== undefined) {
      const took = Math.max(0, entry.endTs - entry.ts);
      row.durationMs += took;
      row.longestMs = Math.max(row.longestMs, took);
    }
  }
  return rows.done();
}

/** Many threads' uses, from the daemon's buckets; `keep` picks the threads. */
export function toolsOfBuckets(buckets: readonly ToolBucket[], keep: (threadId: string) => boolean = () => true): ToolRow[] {
  const rows = new Rows();
  for (const bucket of buckets) {
    if (!keep(bucket.thread_id)) continue;
    const row = rows.row({ kind: bucket.kind, name: bucket.name, server: bucket.server ?? null }, bucket.thread_id);
    row.calls += bucket.calls;
    row.failed += bucket.failed;
    row.declined += bucket.declined;
    row.interrupted += bucket.interrupted;
    row.durationMs += bucket.duration_ms;
    row.longestMs = Math.max(row.longestMs, bucket.longest_ms);
  }
  return rows.done();
}

export type KindFilter = "all" | "commands" | "tools" | "mcp" | "skills";

const KINDS: Record<KindFilter, readonly ToolKind[] | null> = {
  all: null,
  commands: ["command"],
  tools: ["tool", "fileChange", "fileRead", "search"],
  mcp: ["mcp"],
  skills: ["skill"],
};

export const ofKind = (rows: readonly ToolRow[], filter: KindFilter): ToolRow[] =>
  rows.filter((row) => KINDS[filter]?.includes(row.kind) ?? true);

/** MCP uses added up by server, most used first. */
export function byServer(rows: readonly ToolRow[]): { server: string; calls: number; failed: number; tools: number }[] {
  const servers = new Map<string, { server: string; calls: number; failed: number; tools: number }>();
  for (const row of rows) {
    if (row.kind !== "mcp" || !row.server) continue;
    const entry = servers.get(row.server) ?? { server: row.server, calls: 0, failed: 0, tools: 0 };
    entry.calls += row.calls;
    entry.failed += row.failed;
    entry.tools += 1;
    servers.set(row.server, entry);
  }
  return [...servers.values()].sort((a, b) => b.calls - a.calls || a.server.localeCompare(b.server));
}

const KIND_LABEL: Record<ToolKind, string> = {
  command: "Command",
  fileChange: "File edits",
  fileRead: "File reads",
  search: "Searches",
  tool: "Tool",
  mcp: "MCP",
  skill: "Skill",
};

/** How a row is named: kinds counted as a whole by their kind. */
export function rowLabel(row: ToolUse): string {
  if (!row.name) return KIND_LABEL[row.kind];
  if (row.kind === "mcp" && row.server) {
    // Agents name MCP tools after their server; the server is shown apart.
    const short = row.name.split("__").at(-1)!.replace(new RegExp(`^${escape(row.server)}\\.`), "");
    return `${row.server} · ${short}`;
  }
  return row.name;
}

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

export const kindLabel = (kind: ToolKind) => KIND_LABEL[kind];

/** Failed among the uses; null when there were none. */
export const failureShare = (row: ToolRow): number | null => (row.calls === 0 ? null : row.failed / row.calls);
