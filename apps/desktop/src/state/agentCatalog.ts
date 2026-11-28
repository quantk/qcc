// What a thread's agent can be pointed at with `/name`: the skills and slash
// commands it finds by itself in the thread's working directory. Asking means
// running the agent for a moment (Claude Code, Codex) or reading its folders
// (opencode), so answers are kept and looked at again in the background: the
// composer shows what is known at once, and what changed on disk a moment later
// (stale-while-revalidate), at most once per few seconds per thread.

import type { AgentCommand, AgentSkill, ProviderId, ThreadId } from "../api/protocol.gen";

export type AgentCatalog = { skills: readonly AgentSkill[]; commands: readonly AgentCommand[] };

/** Ask the daemon; `refresh` makes it ask the agent instead of using what it kept. */
export type CatalogFetch = (provider: ProviderId, thread: ThreadId, refresh: boolean) => Promise<AgentCatalog>;

const EMPTY: AgentCatalog = { skills: [], commands: [] };

type Entry = {
  /** The latest answer, or the first one on its way. */
  current: Promise<AgentCatalog>;
  /** When the agent was last asked, in ms. */
  asked: number;
  refreshing: boolean;
};

export class AgentCatalogs {
  private entries = new Map<string, Entry>();
  private listeners = new Set<() => void>();

  constructor(
    private readonly fetch: CatalogFetch,
    /** The least time between two looks for one thread, in ms. */
    private readonly interval = 3000,
    private readonly now: () => number = Date.now,
  ) {}

  /** What is known for the thread; the first call asks (the daemon may know already). */
  get(provider: ProviderId, thread: ThreadId): Promise<AgentCatalog> {
    const key = `${provider}\n${thread}`;
    const known = this.entries.get(key);
    if (known) return known.current;
    const current = this.fetch(provider, thread, false).catch(() => {
      // Asked again next time instead of keeping the failure.
      this.entries.delete(key);
      return EMPTY;
    });
    this.entries.set(key, { current, asked: this.now(), refreshing: false });
    return current;
  }

  /**
   * Look again in the background, unless that was done a moment ago or is under
   * way; listeners hear of an answer that differs from what was known.
   */
  revalidate(provider: ProviderId, thread: ThreadId): void {
    const key = `${provider}\n${thread}`;
    const entry = this.entries.get(key);
    if (!entry) {
      void this.get(provider, thread);
      return;
    }
    if (entry.refreshing || this.now() - entry.asked < this.interval) return;
    entry.refreshing = true;
    entry.asked = this.now();
    void Promise.all([entry.current, this.fetch(provider, thread, true)])
      .then(([before, after]) => {
        if (this.entries.get(key) !== entry) return;
        entry.current = Promise.resolve(after);
        if (!sameCatalog(before, after)) this.listeners.forEach((listener) => listener());
      })
      .catch(() => {})
      .finally(() => {
        entry.refreshing = false;
      });
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Forget everything: the daemon went away and may come back with other agents. */
  clear(): void {
    this.entries.clear();
  }
}

function sameCatalog(a: AgentCatalog, b: AgentCatalog): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
