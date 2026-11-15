import { relativePath, tailPath } from "../util/format";
import type { PermissionMode, PluginInfo, ProviderInfo, Thread } from "../api/protocol.gen";

export function providersOf(plugins: readonly PluginInfo[]): ProviderInfo[] {
  return plugins.flatMap((plugin) => plugin.providers);
}

export function environmentsOf(plugins: readonly PluginInfo[]) {
  return plugins.flatMap((plugin) => plugin.environments);
}

export function findProvider(plugins: readonly PluginInfo[], id: string): ProviderInfo | undefined {
  return providersOf(plugins).find((provider) => provider.id === id);
}

/** The directory a thread is about: where it was asked to work. */
export function projectOf(thread: Thread): string {
  return thread.environment?.source_dir ?? thread.cwd;
}

export type Attention = "archived" | "failed" | "waiting" | "active" | "idle";

/** The one thing about a thread its row has to say. */
export function attentionOf(thread: Thread): Attention {
  if (thread.archived) return "archived";
  if ((thread.pending_interactions ?? []).length > 0) return "waiting";
  if (thread.status === "active") return "active";
  if (thread.status === "failed" || thread.last_turn_status === "failed") return "failed";
  return "idle";
}

export const attentionLabel: Record<Attention, string> = {
  archived: "Archived",
  failed: "Failed",
  waiting: "Waiting for your answer",
  active: "Working",
  idle: "Idle",
};

export const modeLabel: Record<PermissionMode, string> = {
  ask: "Ask every time",
  "accept-edits": "Accept edits",
  auto: "Auto review",
  full: "Full access",
};

/** One line saying what the agent last did, or is doing. */
export function activityText(thread: Thread): string {
  if (!thread.activity) return "";
  const { kind, detail, status } = thread.activity;
  // Paths inside the working directory read better without it.
  const path = relativePath(detail, thread.cwd);
  switch (kind) {
    case "agentMessage":
      return detail || (status === "pending" ? "Writing…" : "");
    case "reasoning":
      return status === "pending" ? "Thinking…" : "Thought it over";
    case "plan":
      return detail ? `Plan: ${detail}` : "Planning…";
    case "command":
      return `Run ${detail}`;
    case "fileChange":
      return `Edit ${tailPath(path, 48)}`;
    case "fileRead":
      return `Read ${tailPath(path, 48)}`;
    case "search":
      return `Search ${detail}`;
    case "tool":
    case "extension":
      return detail;
  }
}
