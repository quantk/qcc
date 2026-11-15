// Everything the app knows, in one place. The daemon owns the truth; this store
// mirrors the part of it the window shows and turns user actions into requests.

import { useSyncExternalStore } from "react";

import { type Api, ApiError, type DaemonState, type Notification, codes } from "../api/api";
import type {
  AgentSkill,
  CommandRunParams,
  CommandRunResult,
  DeliverMode,
  InteractionId,
  InteractionResolution,
  MessageContext,
  ModelInfo,
  PermissionMode,
  PluginInfo,
  Project,
  ProjectId,
  ProviderId,
  SendMode,
  Settings,
  SettingsResult,
  SkillStatus,
  Thread,
  ThreadEvent,
  ThreadId,
  ThreadSpawnParams,
} from "../api/protocol.gen";
import { type History, type Place, startHistory, step, visit } from "./history";
import { type Seen, markSeen, markUnseen, seenEverything } from "./reads";
import { visiblePlugins } from "./selectors";
import { type Timeline, applyEvents, emptyTimeline } from "./timeline";

const SHOW_EXAMPLES = "qcc.show-example-plugins";
const SEEN = "qcc.seen";
const COLLAPSED = "qcc.collapsed-families";

/** Null when nothing was ever recorded: the first run of the app. */
function readSeen(): Seen | null {
  try {
    const text = globalThis.localStorage?.getItem(SEEN);
    return text ? (JSON.parse(text) as Seen) : null;
  } catch {
    return null;
  }
}

function saveSeen(seen: Seen) {
  try {
    globalThis.localStorage?.setItem(SEEN, JSON.stringify(seen));
  } catch {
    // Not remembered past this window; it still works until it closes.
  }
}

/** Whether the user can be looking at the window right now. */
function windowInView(): boolean {
  return typeof document === "undefined" || (document.visibilityState === "visible" && document.hasFocus());
}

function readCollapsed(): ReadonlySet<ThreadId> {
  try {
    const ids: unknown = JSON.parse(globalThis.localStorage?.getItem(COLLAPSED) ?? "[]");
    return new Set(Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : []);
  } catch {
    return new Set();
  }
}

function readShowExamples(): boolean {
  try {
    return globalThis.localStorage?.getItem(SHOW_EXAMPLES) === "1";
  } catch {
    return false;
  }
}

/** What a provider said it offers. Asked once per provider and connection. */
export type ModelList = { loading: boolean; models: readonly ModelInfo[]; error: string | null };
export type AgentSkillList = { loading: boolean; skills: readonly AgentSkill[]; error: string | null };

export type AppState = {
  daemon: DaemonState;
  threads: Readonly<Record<ThreadId, Thread>>;
  /** The plugins whose contributions the window offers: example plugins only when
   * `showExamples` is on. */
  plugins: readonly PluginInfo[];
  /** Every plugin the daemon loaded. */
  allPlugins: readonly PluginInfo[];
  showExamples: boolean;
  selected: ThreadId | null;
  /** The app view shown while no thread is selected. Absent: the first one. */
  view: string | null;
  timelines: Readonly<Record<ThreadId, Timeline>>;
  /** The selected thread's events are being read. */
  loading: boolean;
  /** The last thing that went wrong, until dismissed. */
  error: string | null;
  /** Something the user should hear about that is not a failure. */
  notice: string | null;
  models: Readonly<Record<ProviderId, ModelList>>;
  /** Oldest first. */
  projects: readonly Project[];
  /** The daemon's settings and where its programs are. Null until read. */
  settings: SettingsResult | null;
  /** How far the user has seen each thread; see `reads.ts`. */
  seen: Seen;
  /** Where the user has been, for back and forward. */
  history: History;
  /** Threads whose children the sidebar keeps folded under them. */
  collapsed: ReadonlySet<ThreadId>;
  /** The skills plugins offer; whether one is off is in the settings. */
  skills: readonly SkillStatus[];
  /** The skills each agent finds by itself, as it reported them. */
  agentSkills: Readonly<Record<ProviderId, AgentSkillList>>;
};

type Live = { id: number; threadId: ThreadId; ready: boolean; buffer: ThreadEvent[] };

const defaultSchedule = (run: () => void) => {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
  else queueMicrotask(run);
};

export class AppStore {
  private state: AppState = {
    daemon: { status: "connecting" },
    threads: {},
    plugins: [],
    allPlugins: [],
    showExamples: readShowExamples(),
    selected: null,
    view: null,
    timelines: {},
    loading: false,
    error: null,
    notice: null,
    models: {},
    projects: [],
    settings: null,
    seen: readSeen() ?? {},
    history: startHistory,
    collapsed: readCollapsed(),
    skills: [],
    agentSkills: {},
  };
  /** No record existed when the app started: the first listing counts as seen. */
  private firstRun = readSeen() === null;
  private listeners = new Set<() => void>();
  private notifying = false;
  private nextSubscription = 1;
  private watch: number | null = null;
  private live: Live | null = null;

  constructor(
    readonly api: Api,
    /** Coalesces redraws: streamed text arrives far faster than frames. */
    private readonly schedule: (run: () => void) => void = defaultSchedule,
  ) {}

  getState = (): AppState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private set(patch: Partial<AppState>) {
    this.state = { ...this.state, ...patch };
    if (this.notifying) return;
    this.notifying = true;
    this.schedule(() => {
      this.notifying = false;
      for (const listener of this.listeners) listener();
    });
  }

  /** Start mirroring the daemon. Returns a function that stops it. */
  start(): () => void {
    // Coming back to the window shows the thread that was left open.
    const onFocus = () => this.seeSelected();
    globalThis.addEventListener?.("focus", onFocus);
    const stops = [
      () => globalThis.removeEventListener?.("focus", onFocus),
      this.api.onDaemonState((state) => this.daemonChanged(state)),
      this.api.onNotification((notification) => this.notified(notification)),
    ];
    void this.api.daemonState().then((state) => this.daemonChanged(state));
    return () => stops.forEach((stop) => stop());
  }

  private daemonChanged(daemon: DaemonState) {
    const was = this.state.daemon.status;
    this.set({ daemon });
    if (daemon.status === "connected" && was !== "connected") void this.connected();
    if (daemon.status !== "connected") {
      // Subscriptions die with the connection, and a daemon that comes back may
      // have other plugins.
      this.watch = null;
      this.live = null;
      this.set({ models: {}, agentSkills: {} });
    }
  }

  private async connected() {
    const watch = this.nextSubscription++;
    this.watch = watch;
    try {
      const [list, plugins, projects, settings, skills] = await Promise.all([
        this.api.call("thread.watch", { subscription: watch }),
        this.api.call("plugin.list", {}),
        this.api.call("project.list", {}),
        this.api.call("settings.get", {}),
        this.api.call("skill.list", {}),
      ]);
      if (this.watch !== watch) return;
      this.mergeThreads(list.threads);
      this.set({ projects: projects.projects, settings, skills: skills.skills });
      this.set({
        allPlugins: plugins.plugins,
        plugins: visiblePlugins(plugins.plugins, this.state.showExamples),
      });
      if (this.state.selected) await this.follow(this.state.selected);
    } catch (error) {
      this.fail(error);
    }
  }

  private notified(notification: Notification) {
    if (notification.method === "thread.updated") {
      if (notification.params.subscription === this.watch) this.mergeThreads([notification.params.thread]);
    } else if (notification.method === "projects.updated") {
      if (notification.params.subscription === this.watch) this.set({ projects: notification.params.projects });
    } else if (notification.method === "thread.event") {
      const live = this.live;
      if (!live || notification.params.subscription !== live.id) return;
      // Until the backlog is in, live events wait: the fold needs them in order.
      if (live.ready) this.fold(live.threadId, [notification.params.event]);
      else live.buffer.push(notification.params.event);
    }
  }

  /** A projection never goes back in time: updates may overtake the listing. */
  private mergeThreads(threads: readonly Thread[]) {
    const next = { ...this.state.threads };
    for (const thread of threads) {
      const known = next[thread.id];
      if (!known || known.last_seq <= thread.last_seq) next[thread.id] = thread;
    }
    this.set({ threads: next });
    if (this.firstRun) {
      this.firstRun = false;
      this.setSeen(seenEverything(Object.values(next)));
    }
    this.seeSelected();
  }

  /** The thread on screen is seen, as far as it has got, while the window is in view. */
  seeSelected = () => {
    const thread = this.state.selected ? this.state.threads[this.state.selected] : undefined;
    if (thread && windowInView()) this.setSeen(markSeen(this.state.seen, thread));
  };

  private setSeen(seen: Seen) {
    if (seen === this.state.seen) return;
    this.set({ seen });
    saveSeen(seen);
  }

  markUnread = (threadId: ThreadId) => {
    const thread = this.state.threads[threadId];
    if (thread) this.setSeen(markUnseen(this.state.seen, thread));
  };

  markRead = (threadId: ThreadId) => {
    const thread = this.state.threads[threadId];
    if (thread) this.setSeen(markSeen(this.state.seen, thread));
  };

  private fold(threadId: ThreadId, events: readonly ThreadEvent[]) {
    if (events.length === 0) return;
    const timeline = applyEvents(this.state.timelines[threadId] ?? emptyTimeline, events);
    this.set({ timelines: { ...this.state.timelines, [threadId]: timeline } });
  }

  private unfollow() {
    const live = this.live;
    this.live = null;
    if (live) void this.api.call("thread.unsubscribe", { subscription: live.id }).catch(() => {});
  }

  private async follow(threadId: ThreadId) {
    this.unfollow();
    const live: Live = { id: this.nextSubscription++, threadId, ready: false, buffer: [] };
    this.live = live;
    this.set({ loading: !this.state.timelines[threadId] });
    try {
      const after = this.state.timelines[threadId]?.lastSeq ?? 0;
      const { events } = await this.api.call("thread.events", {
        thread_id: threadId,
        after,
        subscription: live.id,
      });
      this.fold(threadId, events);
      if (this.live !== live) return;
      this.fold(threadId, live.buffer);
      live.buffer = [];
      live.ready = true;
      if (!this.state.timelines[threadId]) {
        this.set({ timelines: { ...this.state.timelines, [threadId]: emptyTimeline } });
      }
    } catch (error) {
      if (this.live === live) this.live = null;
      this.fail(error);
    } finally {
      if (this.live === live || this.live === null) this.set({ loading: false });
    }
  }

  private fail(error: unknown) {
    // A lost connection is already on screen as the daemon's state.
    if (error instanceof ApiError && error.code === codes.notConnected) return;
    this.set({ error: error instanceof Error ? error.message : String(error) });
  }

  private async attempt<T>(action: () => Promise<T>): Promise<T | null> {
    try {
      return await action();
    } catch (error) {
      this.fail(error);
      return null;
    }
  }

  // ----- actions ------------------------------------------------------------------

  dismissError = () => this.set({ error: null });

  notify = (notice: string) => this.set({ notice });

  dismissNotice = () => this.set({ notice: null });

  /** Run a plugin command. Its refusal comes back for the dialog that asked. */
  async runCommand(params: CommandRunParams): Promise<CommandRunResult | { error: string }> {
    try {
      return await this.api.call("command.run", params);
    } catch (error) {
      return { error: error instanceof Error ? error.message : String(error) };
    }
  }

  /** Ask the provider which models it offers, unless that is known or underway. */
  loadModels = async (provider: ProviderId, refresh = false) => {
    const known = this.state.models[provider];
    if (known?.loading || (known && !refresh && !known.error)) return;
    const put = (list: ModelList) => this.set({ models: { ...this.state.models, [provider]: list } });
    put({ loading: true, models: known?.models ?? [], error: null });
    try {
      const { models } = await this.api.call("provider.models", { provider, refresh });
      put({ loading: false, models, error: null });
    } catch (error) {
      put({ loading: false, models: [], error: error instanceof Error ? error.message : String(error) });
    }
  };

  /** Ask the agent which skills it finds by itself, unless that is known or underway. */
  loadAgentSkills = async (provider: ProviderId, refresh = false) => {
    const known = this.state.agentSkills[provider];
    if (known?.loading || (known && !refresh && !known.error)) return;
    const put = (list: AgentSkillList) => this.set({ agentSkills: { ...this.state.agentSkills, [provider]: list } });
    put({ loading: true, skills: known?.skills ?? [], error: null });
    try {
      const { skills } = await this.api.call("provider.skills", { provider, refresh });
      put({ loading: false, skills, error: null });
    } catch (error) {
      put({ loading: false, skills: [], error: error instanceof Error ? error.message : String(error) });
    }
  };

  /** Offer example plugins (the echo provider and its views) or leave them out. */
  setShowExamples = (show: boolean) => {
    try {
      localStorage.setItem(SHOW_EXAMPLES, show ? "1" : "0");
    } catch {
      // Not remembered; it still applies to this window.
    }
    this.set({ showExamples: show, plugins: visiblePlugins(this.state.allPlugins, show) });
  };

  /** Fold the threads a thread started under it in the sidebar, or unfold them. */
  toggleCollapsed = (threadId: ThreadId) => {
    const collapsed = new Set(this.state.collapsed);
    if (!collapsed.delete(threadId)) collapsed.add(threadId);
    // Threads that are gone need not be remembered.
    for (const id of collapsed) if (!this.state.threads[id]) collapsed.delete(id);
    try {
      globalThis.localStorage?.setItem(COLLAPSED, JSON.stringify([...collapsed]));
    } catch {
      // Not remembered past this window.
    }
    this.set({ collapsed });
  };

  select = (threadId: ThreadId | null) => {
    this.go({ thread: threadId, view: this.state.view }, true);
  };

  /** Leave the selected thread for one of the app's own screens. */
  openView = (view: string) => {
    this.go({ thread: null, view }, true);
  };

  /** Go back (`-1`) or forward (`1`) through the places visited. */
  navigate = (by: -1 | 1) => {
    const next = step(this.state.history, by);
    if (!next) return;
    this.set({ history: next.history });
    this.go(next.place, false);
  };

  private go(place: Place, record: boolean) {
    if (record) this.set({ history: visit(this.state.history, place) });
    if (place.view !== this.state.view) this.set({ view: place.view });
    const threadId = place.thread;
    if (threadId === this.state.selected) return;
    this.set({ selected: threadId, loading: false });
    this.seeSelected();
    if (threadId === null) this.unfollow();
    else if (this.state.daemon.status === "connected") void this.follow(threadId);
  }

  async spawn(params: ThreadSpawnParams): Promise<Thread | null> {
    const result = await this.attempt(() => this.api.call("thread.spawn", params));
    if (!result) return null;
    this.mergeThreads([result.thread]);
    this.select(result.thread.id);
    return result.thread;
  }

  /** Resolves to whether the daemon took the message. */
  /** `context` says what the message is about; the agent is told in a note. */
  async send(
    threadId: ThreadId,
    text: string,
    delivery: SendMode,
    context: MessageContext[] = [],
  ): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.send", { thread_id: threadId, text, delivery, context }),
    );
    return result !== null;
  }

  /** Take a queued message back. Resolves to its text, or null if it was gone. */
  async unqueue(threadId: ThreadId, queueId: string): Promise<string | null> {
    const result = await this.attempt(() =>
      this.api.call("thread.unqueue", { thread_id: threadId, queue_id: queueId }),
    );
    return result?.text ?? null;
  }

  /** Start the thread's conversation afresh. Resolves to whether the daemon took it. */
  async resetConversation(threadId: ThreadId): Promise<boolean> {
    const result = await this.attempt(() => this.api.call("thread.reset", { thread_id: threadId }));
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Ask the agent to compact its context. Resolves to whether the daemon took it. */
  async compact(threadId: ThreadId): Promise<boolean> {
    const result = await this.attempt(() => this.api.call("thread.compact", { thread_id: threadId }));
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  stop(threadId: ThreadId) {
    return this.attempt(() => this.api.call("thread.stop", { thread_id: threadId, mode: "interrupt" }));
  }

  resolve(threadId: ThreadId, interactionId: InteractionId, resolution: InteractionResolution) {
    return this.attempt(() =>
      this.api.call("interaction.resolve", {
        thread_id: threadId,
        interaction_id: interactionId,
        resolution,
      }),
    );
  }

  async setMode(threadId: ThreadId, mode: PermissionMode | null) {
    const result = await this.attempt(() =>
      this.api.call("thread.set_mode", { thread_id: threadId, permission_mode: mode }),
    );
    if (result) this.mergeThreads([result.thread]);
  }

  /** Bring a worktree's work back to its source. Resolves to what was done, or null. */
  async deliver(threadId: ThreadId, mode: DeliverMode): Promise<string | null> {
    const result = await this.attempt(() => this.api.call("thread.deliver", { thread_id: threadId, mode }));
    if (result) this.notify(result.message);
    return result?.message ?? null;
  }

  /** Move a thread into a worktree of its own. Resolves to whether the daemon took it. */
  async isolate(threadId: ThreadId, environment: string, carryChanges: boolean): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.isolate", { thread_id: threadId, environment, carry_changes: carryChanges }),
    );
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Resolves to whether the daemon took the name. */
  async rename(threadId: ThreadId, title: string): Promise<boolean> {
    const result = await this.attempt(() => this.api.call("thread.rename", { thread_id: threadId, title }));
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Resolves to whether the thread now has that agent, model and effort. */
  async setAgent(
    threadId: ThreadId,
    provider: ProviderId,
    model: string | null,
    effort: string | null,
  ): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.set_agent", { thread_id: threadId, provider, model, effort }),
    );
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Read the settings again: a program may have been installed meanwhile. */
  async reloadSettings() {
    const settings = await this.attempt(() => this.api.call("settings.get", {}));
    if (settings) this.set({ settings });
  }

  /** Resolves to whether the daemon took them; its refusal is shown as the error. */
  async saveSettings(settings: Settings): Promise<boolean> {
    const result = await this.attempt(() => this.api.call("settings.set", { settings }));
    if (result) this.set({ settings: result, models: {} });
    return result !== null;
  }

  /**
   * Change one part of the settings (MCP servers, skills). The settings are read
   * again first, so neither a change from the CLI nor an unsaved draft of the
   * settings screen is lost. Resolves to whether the daemon took it.
   */
  async changeSettings(change: (settings: Settings) => Settings): Promise<boolean> {
    const current = await this.attempt(() => this.api.call("settings.get", {}));
    if (!current) return false;
    const result = await this.attempt(() => this.api.call("settings.set", { settings: change(current.settings) }));
    if (result) this.set({ settings: result });
    return result !== null;
  }

  /** Resolves to the project, or null when the daemon refused (the error is shown). */
  async saveProject(id: ProjectId | null, name: string, folders: string[]): Promise<Project | null> {
    const result = await this.attempt(() =>
      id
        ? this.api.call("project.update", { id, name, folders })
        : this.api.call("project.create", { name, folders }),
    );
    if (!result) return null;
    const others = this.state.projects.filter((project) => project.id !== result.project.id);
    const projects = id ? this.state.projects.map((p) => (p.id === id ? result.project : p)) : [...others, result.project];
    this.set({ projects });
    return result.project;
  }

  async deleteProject(id: ProjectId) {
    const done = await this.attempt(() => this.api.call("project.delete", { id }));
    if (done) this.set({ projects: this.state.projects.filter((project) => project.id !== id) });
  }

  /** Turn a skill on or off for this thread alone. Resolves to whether the daemon took it. */
  async setThreadSkill(threadId: ThreadId, skill: string, enabled: boolean): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.set_skill", { thread_id: threadId, skill, enabled }),
    );
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Turn an MCP server on or off for this thread alone. Resolves to whether the daemon took it. */
  async setThreadMcp(threadId: ThreadId, server: string, enabled: boolean): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.set_mcp", { thread_id: threadId, server, enabled }),
    );
    if (result) this.mergeThreads([result.thread]);
    return result !== null;
  }

  /** Place a thread under another, or let it stand on its own. */
  async setParent(threadId: ThreadId, parent: ThreadId | null) {
    const result = await this.attempt(() =>
      this.api.call("thread.set_parent", { thread_id: threadId, parent }),
    );
    if (result) this.mergeThreads([result.thread]);
  }

  async setProject(threadId: ThreadId, project: ProjectId | null) {
    const result = await this.attempt(() =>
      this.api.call("thread.set_project", { thread_id: threadId, project }),
    );
    if (result) this.mergeThreads([result.thread]);
  }

  /**
   * Archive a thread. When its environment holds work that would be lost, nothing
   * is archived and the reason comes back, for the user to confirm with `force`.
   */
  async archive(threadId: ThreadId, force: boolean): Promise<{ refused: string } | null> {
    try {
      const { thread } = await this.api.call("thread.archive", { thread_id: threadId, force });
      this.mergeThreads([thread]);
      return null;
    } catch (error) {
      if (!force && error instanceof ApiError && error.code === codes.environmentError) {
        return { refused: error.message };
      }
      this.fail(error);
      return null;
    }
  }
}

let appStore: AppStore | null = null;

export function provideStore(store: AppStore) {
  appStore = store;
}

export function store(): AppStore {
  if (!appStore) throw new Error("the store is not set up");
  return appStore;
}

/** Subscribe a component to a slice of the state. The slice must be stable. */
export function useApp<T>(select: (state: AppState) => T): T {
  const current = store();
  return useSyncExternalStore(current.subscribe, () => select(current.getState()));
}
