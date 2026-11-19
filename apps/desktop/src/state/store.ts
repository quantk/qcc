// Everything the app knows, in one place. The daemon owns the truth; this store
// mirrors the part of it the window shows and turns user actions into requests.

import { useSyncExternalStore } from "react";

import { type Api, ApiError, type DaemonState, type Notification, codes } from "../api/api";
import type {
  AgentSkill,
  AutomationEntry,
  AutomationId,
  AutomationRun,
  AutomationSpec,
  CommandRunParams,
  CommandRunResult,
  CommentAddParams,
  CommentId,
  DeliverMode,
  InteractionId,
  InteractionResolution,
  LineComment,
  MessageContext,
  ModelInfo,
  NewAttachment,
  PermissionMode,
  PluginInfo,
  Project,
  ProjectId,
  ProviderId,
  ProviderLimits,
  ProviderLimitsError,
  SendMode,
  Settings,
  SettingsResult,
  SkillStatus,
  Thread,
  ThreadEvent,
  ThreadId,
  ThreadSpawnParams,
  ThreadUnqueueResult,
} from "../api/protocol.gen";
import { type Stayed, stayedAfter } from "./archive";
import { withAutomation } from "./automations";
import { type Layout, parseLayout } from "./blocks";
import { type History, type Place, startHistory, step, visit } from "./history";
import { limitProviders, mergeLimits, wantsRefresh } from "./limits";
import { SIDEBAR, SIDE_PANEL_WIDTH, clampWidth } from "./layout";
import { type Seen, markSeen, markUnseen, seenEverything } from "./reads";
import { groupKeys, pruneFolded, withFolded } from "./groups";
import { projectGroupKey, visiblePlugins, withPlugin } from "./selectors";
import { errorMessage } from "../util/format";
import { readJson, readText, saveJson, saveText } from "../util/storage";
import { type Timeline, applyEvents, emptyTimeline } from "./timeline";

const SHOW_EXAMPLES = "qcc.show-example-plugins";
const SEEN = "qcc.seen";
const COLLAPSED = "qcc.collapsed-families";
const FOLDED_GROUPS = "qcc.collapsed-groups";
const SIDE_PANEL = "qcc.side-panel";
const SIDEBAR_WIDTH = "qcc.sidebar-width";
const BLOCK_LAYOUT = "qcc.block-layout";

/** The panel at the right of a thread: whether it is open, which tab it shows, how wide it is. */
export type SidePanel = { open: boolean; tab: string | null; width: number };

/**
 * Narrower than this, the side panel would squeeze the thread: it covers the thread
 * when asked for instead, and is closed until then.
 */
export const SIDE_PANEL_BESIDE = "(min-width: 1100px)";

function panelFitsBeside(): boolean {
  return globalThis.matchMedia?.(SIDE_PANEL_BESIDE).matches ?? true;
}

function readSidePanel(): SidePanel {
  const saved = readJson(SIDE_PANEL);
  if (saved && typeof saved === "object") {
    const { open, tab, width } = saved as Record<string, unknown>;
    return { open: open !== false, tab: typeof tab === "string" ? tab : null, width: clampWidth(width, SIDE_PANEL_WIDTH) };
  }
  return { open: true, tab: null, width: SIDE_PANEL_WIDTH.initial };
}

/** Null when nothing was ever recorded: the first run of the app. */
function readSeen(): Seen | null {
  return readJson(SEEN) as Seen | null;
}

/** Whether the user can be looking at the window right now. */
function windowInView(): boolean {
  return typeof document === "undefined" || (document.visibilityState === "visible" && document.hasFocus());
}

function readStringSet(key: string): ReadonlySet<string> {
  const ids = readJson(key);
  return new Set(Array.isArray(ids) ? ids.filter((id): id is string => typeof id === "string") : []);
}

/** What a provider said it offers. Asked once per provider and connection. */
export type ProviderList<T> = { loading: boolean; items: readonly T[]; error: string | null };
export type ModelList = ProviderList<ModelInfo>;
export type AgentSkillList = ProviderList<AgentSkill>;

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
  /** Oldest first, each with its next and latest run. */
  automations: readonly AutomationEntry[];
  /** The daemon's settings and where its programs are. Null until read. */
  settings: SettingsResult | null;
  /** How far the user has seen each thread; see `reads.ts`. */
  seen: Seen;
  /** Where the user has been, for back and forward. */
  history: History;
  /** Threads whose children the sidebar keeps folded under them. */
  collapsed: ReadonlySet<ThreadId>;
  /** Sidebar groups (`ThreadGroup.key`) that show only their heading. */
  foldedGroups: ReadonlySet<string>;
  sidePanel: SidePanel;
  /** The tab chosen in the selected thread's view, by block id. Null: its first tab.
   * Not remembered: another thread opens on its first tab. */
  threadTab: string | null;
  /** How wide the thread list at the left is, in pixels. */
  sidebarWidth: number;
  /** Where the user put blocks, over their default places; see `blocks.ts`. */
  blockLayout: Layout;
  /** The side panel covers the thread in a narrow window. Not remembered. */
  sideOverlay: boolean;
  /** The skills plugins offer; whether one is off is in the settings. */
  skills: readonly SkillStatus[];
  /** The skills each agent finds by itself, as it reported them. */
  agentSkills: Readonly<Record<ProviderId, AgentSkillList>>;
  /** Comments on each thread's changes, resolved ones too, oldest first. Read for a
   * thread when a view asks (`loadComments`), then kept current by the daemon. */
  comments: Readonly<Record<ThreadId, readonly LineComment[]>>;
  /** The latest usage limits each provider's account reported; see `limits.ts`. */
  limits: Readonly<Record<ProviderId, ProviderLimits>>;
  /** The agents are being asked for their limits now. */
  limitsRefreshing: boolean;
  /** Why the last time they were asked some could not tell. */
  limitErrors: readonly ProviderLimitsError[];
};

type Live = { id: number; threadId: ThreadId; ready: boolean; buffer: ThreadEvent[] };

/** A thread whose events someone wants as they come: how many, and the subscription. */
type Followed = { holders: number; live: Live | null };

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
    showExamples: readText(SHOW_EXAMPLES) === "1",
    selected: null,
    view: null,
    timelines: {},
    loading: false,
    error: null,
    notice: null,
    models: {},
    projects: [],
    automations: [],
    settings: null,
    seen: readSeen() ?? {},
    history: startHistory,
    collapsed: readStringSet(COLLAPSED),
    foldedGroups: readStringSet(FOLDED_GROUPS),
    sidePanel: readSidePanel(),
    threadTab: null,
    sidebarWidth: clampWidth(readJson(SIDEBAR_WIDTH), SIDEBAR),
    blockLayout: parseLayout(readJson(BLOCK_LAYOUT)),
    sideOverlay: false,
    skills: [],
    agentSkills: {},
    comments: {},
    limits: {},
    limitsRefreshing: false,
    limitErrors: [],
  };
  /** No record existed when the app started: the first listing counts as seen. */
  private firstRun = readSeen() === null;
  private listeners = new Set<() => void>();
  private notifying = false;
  private nextSubscription = 1;
  private watch: number | null = null;
  /** Threads whose timelines are kept current: the selected one and whatever a view holds. */
  private followed = new Map<ThreadId, Followed>();
  /** Attachments read to be shown, by path: they never change. */
  private attachmentUrls = new Map<string, Promise<string | null>>();

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
      for (const entry of this.followed.values()) entry.live = null;
      this.set({ models: {}, agentSkills: {} });
    }
  }

  private async connected() {
    const watch = this.nextSubscription++;
    this.watch = watch;
    try {
      const [list, plugins, projects, settings, skills, automations, limits] = await Promise.all([
        this.api.call("thread.watch", { subscription: watch }),
        this.api.call("plugin.list", {}),
        this.api.call("project.list", {}),
        this.api.call("settings.get", {}),
        this.api.call("skill.list", {}),
        this.api.call("automation.list", {}),
        this.api.call("provider.limits", {}),
      ]);
      if (this.watch !== watch) return;
      this.mergeThreads(list.threads);
      this.set({
        projects: projects.projects,
        settings,
        skills: skills.skills,
        automations: automations.automations,
        limits: mergeLimits(this.state.limits, limits.limits),
      });
      this.setPlugins(plugins.plugins);
      // What the daemon remembers may be old: ask the agents, without holding up the rest.
      if (wantsRefresh(limitProviders(this.state.allPlugins), this.state.limits, Date.now())) {
        void this.refreshLimits();
      }
      await Promise.all([...this.followed.keys()].map((threadId) => this.followLive(threadId)));
    } catch (error) {
      this.fail(error);
    }
  }

  private notified(notification: Notification) {
    if (notification.method === "thread.updated") {
      if (notification.params.subscription === this.watch) this.mergeThreads([notification.params.thread]);
    } else if (notification.method === "plugins.updated") {
      if (notification.params.subscription === this.watch) this.pluginsChanged(notification.params.plugins);
    } else if (notification.method === "projects.updated") {
      if (notification.params.subscription === this.watch) this.set({ projects: notification.params.projects });
    } else if (notification.method === "automations.updated") {
      if (notification.params.subscription === this.watch) this.set({ automations: notification.params.automations });
    } else if (notification.method === "limits.updated") {
      const { subscription, limits } = notification.params;
      if (subscription === this.watch) this.set({ limits: mergeLimits(this.state.limits, [limits]) });
    } else if (notification.method === "comments.updated") {
      const { subscription, thread_id, comments } = notification.params;
      if (subscription === this.watch) this.setComments(thread_id, comments);
    } else if (notification.method === "thread.event") {
      const { subscription } = notification.params;
      const live = [...this.followed.values()].find((entry) => entry.live?.id === subscription)?.live;
      if (!live) return;
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
    saveJson(SEEN, seen);
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

  /**
   * Keep a thread's timeline current until the returned function is called. Holds
   * are counted: the thread stays followed while anyone holds it, and is followed
   * again after the daemon comes back.
   */
  hold = (threadId: ThreadId): (() => void) => {
    const entry = this.followed.get(threadId);
    if (entry) entry.holders += 1;
    else {
      this.followed.set(threadId, { holders: 1, live: null });
      if (this.state.daemon.status === "connected") void this.followLive(threadId);
    }
    let held = true;
    return () => {
      if (!held) return;
      held = false;
      this.release(threadId);
    };
  };

  private release(threadId: ThreadId) {
    const entry = this.followed.get(threadId);
    if (!entry) return;
    entry.holders -= 1;
    if (entry.holders > 0) return;
    this.followed.delete(threadId);
    const live = entry.live;
    if (live) void this.api.call("thread.unsubscribe", { subscription: live.id }).catch(() => {});
  }

  /** The selected thread's hold. */
  private selectedHold: (() => void) | null = null;

  private async followLive(threadId: ThreadId) {
    const entry = this.followed.get(threadId);
    if (!entry) return;
    const live: Live = { id: this.nextSubscription++, threadId, ready: false, buffer: [] };
    entry.live = live;
    const current = () => this.followed.get(threadId)?.live === live;
    const shown = () => this.state.selected === threadId;
    if (shown()) this.set({ loading: !this.state.timelines[threadId] });
    try {
      const after = this.state.timelines[threadId]?.lastSeq ?? 0;
      const { events } = await this.api.call("thread.events", {
        thread_id: threadId,
        after,
        subscription: live.id,
      });
      this.fold(threadId, events);
      if (!current()) {
        // Let go while the backlog was on its way: the daemon is still sending.
        void this.api.call("thread.unsubscribe", { subscription: live.id }).catch(() => {});
        return;
      }
      this.fold(threadId, live.buffer);
      live.buffer = [];
      live.ready = true;
      if (!this.state.timelines[threadId]) {
        this.set({ timelines: { ...this.state.timelines, [threadId]: emptyTimeline } });
      }
    } catch (error) {
      if (current()) entry.live = null;
      this.fail(error);
    } finally {
      if (shown()) this.set({ loading: false });
    }
  }

  private fail(error: unknown) {
    // A lost connection is already on screen as the daemon's state.
    if (error instanceof ApiError && error.code === codes.notConnected) return;
    this.set({ error: errorMessage(error) });
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
      return { error: errorMessage(error) };
    }
  }

  /** Ask the provider which models it offers, unless that is known or underway. */
  loadModels = (provider: ProviderId, refresh = false) =>
    this.loadList(
      () => this.state.models[provider],
      (list) => this.set({ models: { ...this.state.models, [provider]: list } }),
      refresh,
      async () => (await this.api.call("provider.models", { provider, refresh })).models,
    );

  /** Ask the agent which skills it finds by itself, unless that is known or underway. */
  loadAgentSkills = (provider: ProviderId, refresh = false) =>
    this.loadList(
      () => this.state.agentSkills[provider],
      (list) => this.set({ agentSkills: { ...this.state.agentSkills, [provider]: list } }),
      refresh,
      async () => (await this.api.call("provider.skills", { provider, refresh })).skills,
    );

  /**
   * Ask the agents how much of their accounts' usage limits is spent: each runs for a
   * moment, without a prompt. Answers also come as `limits.updated`.
   */
  refreshLimits = async () => {
    if (this.state.limitsRefreshing) return;
    this.set({ limitsRefreshing: true });
    try {
      const result = await this.api.call("provider.limits", { refresh: true });
      this.set({ limits: mergeLimits(this.state.limits, result.limits), limitErrors: result.errors ?? [] });
    } catch (error) {
      this.fail(error);
    } finally {
      this.set({ limitsRefreshing: false });
    }
  };

  /** Fill one of the lists a provider is asked for, unless it is known or underway. */
  private async loadList<T>(
    known: () => ProviderList<T> | undefined,
    put: (list: ProviderList<T>) => void,
    refresh: boolean,
    fetch: () => Promise<readonly T[]>,
  ) {
    const list = known();
    if (list?.loading || (list && !refresh && !list.error)) return;
    put({ loading: true, items: list?.items ?? [], error: null });
    try {
      put({ loading: false, items: await fetch(), error: null });
    } catch (error) {
      put({ loading: false, items: [], error: errorMessage(error) });
    }
  }

  private setPlugins(allPlugins: readonly PluginInfo[]) {
    this.set({ allPlugins, plugins: visiblePlugins(allPlugins, this.state.showExamples) });
  }

  /** A plugin started or stopped: what it offered comes or goes, and with it the
   * programs and skills the settings list. */
  private pluginsChanged(allPlugins: readonly PluginInfo[]) {
    this.setPlugins(allPlugins);
    void this.reloadSettings();
    void this.attempt(() => this.api.call("skill.list", {})).then((skills) => {
      if (skills) this.set({ skills: skills.skills });
    });
  }

  /**
   * Turn a plugin on or off. Turning off one whose agents are working is refused
   * unless `force`: that refusal comes back as `busy`, for the user to confirm,
   * rather than as the error.
   */
  setPluginEnabled = async (id: string, enabled: boolean, force = false): Promise<{ ok: boolean; busy?: string }> => {
    try {
      const result = enabled
        ? await this.api.call("plugin.enable", { id })
        : await this.api.call("plugin.disable", { id, force });
      this.pluginsChanged(withPlugin(this.state.allPlugins, result.plugin));
      return { ok: true };
    } catch (error) {
      if (!enabled && !force && error instanceof ApiError && error.code === codes.threadBusy) {
        return { ok: false, busy: error.message };
      }
      this.fail(error);
      return { ok: false };
    }
  };

  /** Offer example plugins (the echo provider and its views) or leave them out. */
  setShowExamples = (show: boolean) => {
    saveText(SHOW_EXAMPLES, show ? "1" : "0");
    this.set({ showExamples: show, plugins: visiblePlugins(this.state.allPlugins, show) });
  };

  /** Open or close the panel at the right of a thread, or show one of its tabs. */
  setSidePanel = (patch: Partial<SidePanel>) => {
    const sidePanel = { ...this.state.sidePanel, ...patch };
    saveJson(SIDE_PANEL, sidePanel);
    this.set({ sidePanel });
  };

  /** Resize the thread list. `save` remembers the width; a drag saves once, when it ends. */
  setSidebarWidth = (width: number, save = true) => {
    const sidebarWidth = clampWidth(width, SIDEBAR);
    if (save) saveJson(SIDEBAR_WIDTH, sidebarWidth);
    if (sidebarWidth !== this.state.sidebarWidth) this.set({ sidebarWidth });
  };

  /** Change where blocks stand; remembered for the next start. */
  setBlockLayout = (blockLayout: Layout) => {
    saveJson(BLOCK_LAYOUT, blockLayout);
    this.set({ blockLayout });
  };

  /** Resize the side panel; `save` as for the sidebar. */
  setSidePanelWidth = (width: number, save = true) => {
    const sidePanel = { ...this.state.sidePanel, width: clampWidth(width, SIDE_PANEL_WIDTH) };
    if (save) saveJson(SIDE_PANEL, sidePanel);
    if (sidePanel.width !== this.state.sidePanel.width) this.set({ sidePanel });
  };

  /** Show a tab of the selected thread's view, by its block's id. */
  openThreadTab = (tab: string | null) => {
    if (tab !== this.state.threadTab) this.set({ threadTab: tab });
  };

  /** Show the side panel, wherever it fits, on a tab of it. */
  openSidePanel = (tab: string) => {
    this.setSidePanel({ tab, ...(panelFitsBeside() ? { open: true } : {}) });
    if (!panelFitsBeside()) this.set({ sideOverlay: true });
  };

  toggleSidePanel = () => {
    if (panelFitsBeside()) this.setSidePanel({ open: !this.state.sidePanel.open });
    else this.set({ sideOverlay: !this.state.sideOverlay });
  };

  /** Fold the threads a thread started under it in the sidebar, or unfold them. */
  toggleCollapsed = (threadId: ThreadId) => {
    const collapsed = new Set(this.state.collapsed);
    if (!collapsed.delete(threadId)) collapsed.add(threadId);
    // Threads that are gone need not be remembered.
    for (const id of collapsed) if (!this.state.threads[id]) collapsed.delete(id);
    saveJson(COLLAPSED, [...collapsed]);
    this.set({ collapsed });
  };

  /** Fold the sidebar groups with these keys, or unfold them. */
  foldGroups = (keys: readonly string[], fold: boolean) => {
    let folded = withFolded(this.state.foldedGroups, keys, fold);
    // Groups that are gone need not be remembered; until the daemon has said which
    // there are, every one is kept.
    if (this.state.daemon.status === "connected") folded = pruneFolded(folded, groupKeys(this.state.threads, this.state.projects));
    saveJson(FOLDED_GROUPS, [...folded]);
    this.set({ foldedGroups: folded });
  };

  toggleGroup = (key: string) => this.foldGroups([key], !this.state.foldedGroups.has(key));

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
    this.set({ selected: threadId, loading: false, threadTab: null });
    this.selectedHold?.();
    this.selectedHold = threadId === null ? null : this.hold(threadId);
    this.seeSelected();
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
    attachments: NewAttachment[] = [],
  ): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.send", { thread_id: threadId, text, delivery, context, attachments }),
    );
    return result !== null;
  }

  // ----- comments on changes ----------------------------------------------------

  private setComments(threadId: ThreadId, comments: readonly LineComment[]) {
    this.set({ comments: { ...this.state.comments, [threadId]: comments } });
  }

  /** Read the comments on a thread's changes; the daemon keeps them current after. */
  async loadComments(threadId: ThreadId): Promise<void> {
    const result = await this.attempt(() =>
      this.api.call("comment.list", { thread_id: threadId, include_resolved: true }),
    );
    if (result) this.setComments(threadId, result.comments);
  }

  /** Resolves to whether the daemon kept it. */
  async addComment(params: CommentAddParams): Promise<boolean> {
    const result = await this.attempt(() => this.api.call("comment.add", params));
    if (!result) return false;
    const known = this.state.comments[params.thread_id] ?? [];
    if (!known.some((comment) => comment.id === result.comment.id)) {
      this.setComments(params.thread_id, [...known, result.comment]);
    }
    return true;
  }

  async resolveComments(threadId: ThreadId, ids: CommentId[], reopen = false): Promise<void> {
    const result = await this.attempt(() => this.api.call("comment.resolve", { comment_ids: ids, reopen }));
    if (!result) return;
    const changed = new Map(result.comments.map((comment) => [comment.id, comment]));
    const known = this.state.comments[threadId] ?? [];
    this.setComments(threadId, known.map((comment) => changed.get(comment.id) ?? comment));
  }

  async deleteComment(threadId: ThreadId, id: CommentId): Promise<void> {
    const result = await this.attempt(() => this.api.call("comment.delete", { comment_id: id }));
    if (!result) return;
    const known = this.state.comments[threadId] ?? [];
    this.setComments(threadId, known.filter((comment) => comment.id !== id));
  }

  /** Take a queued message back. Resolves to its text, or null if it was gone. */
  /** The message taken back, or null if it was not. */
  async unqueue(threadId: ThreadId, queueId: string): Promise<ThreadUnqueueResult | null> {
    return this.attempt(() => this.api.call("thread.unqueue", { thread_id: threadId, queue_id: queueId }));
  }

  /** A data URL of one of the thread's attachments, to show it. Read once per path. */
  attachmentUrl(threadId: ThreadId, path: string): Promise<string | null> {
    const known = this.attachmentUrls.get(path);
    if (known) return known;
    const read = this.api
      .call("thread.attachment", { thread_id: threadId, path })
      .then(({ mime, data }) => `data:${mime};base64,${data}`)
      .catch(() => {
        // Not kept: the next look asks again.
        this.attachmentUrls.delete(path);
        return null;
      });
    this.attachmentUrls.set(path, read);
    return read;
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

  /**
   * Make an automation (no `id`) or replace what it is. The daemon's refusal (a
   * schedule it cannot read, a directory that is not there) comes back for the dialog.
   */
  async saveAutomation(id: AutomationId | null, spec: AutomationSpec): Promise<AutomationEntry | { error: string }> {
    try {
      const { automation } = id
        ? await this.api.call("automation.update", { id, spec })
        : await this.api.call("automation.create", spec);
      this.putAutomation(automation);
      return automation;
    } catch (error) {
      return { error: errorMessage(error) };
    }
  }

  async setAutomationEnabled(id: AutomationId, enabled: boolean) {
    const result = await this.attempt(() => this.api.call("automation.set_enabled", { id, enabled }));
    if (result) this.putAutomation(result.automation);
  }

  async deleteAutomation(id: AutomationId) {
    const done = await this.attempt(() => this.api.call("automation.delete", { id }));
    if (done) this.set({ automations: this.state.automations.filter((entry) => entry.automation.id !== id) });
  }

  /** Run an automation now. Resolves to the run, which may have been skipped. */
  async runAutomation(id: AutomationId): Promise<AutomationRun | null> {
    const result = await this.attempt(() => this.api.call("automation.run", { id }));
    if (!result) return null;
    const known = this.state.automations.find((entry) => entry.automation.id === id);
    if (known) this.putAutomation({ ...known, last_run: result.run });
    return result.run;
  }

  /** An automation's latest runs, newest first; null when they cannot be read (the error is shown). */
  automationRuns(id: AutomationId, limit?: number): Promise<AutomationRun[] | null> {
    return this.attempt(async () => (await this.api.call("automation.runs", { id, limit: limit ?? null })).runs);
  }

  private putAutomation(entry: AutomationEntry) {
    this.set({ automations: withAutomation(this.state.automations, entry) });
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
    if (result) {
      this.mergeThreads([result.thread]);
      // Moved into a folded project, it would vanish; unfold it so it is seen landing.
      if (project && this.state.foldedGroups.has(projectGroupKey(project))) this.foldGroups([projectGroupKey(project)], false);
    }
  }

  /**
   * Archive a thread with every thread under it. Threads that stayed (refused, or
   * kept for a refusal under them) come back with the reason, for the user to see
   * and confirm with `force`; null when everything went or the call failed.
   */
  async archive(threadId: ThreadId, force: boolean): Promise<Stayed[] | null> {
    try {
      const result = await this.api.call("thread.archive", { thread_id: threadId, force, descendants: true });
      this.mergeThreads([result.thread, ...(result.archived ?? [])]);
      return stayedAfter(result, this.state.threads);
    } catch (error) {
      this.fail(error);
      return null;
    }
  }

  /**
   * Which of `threads` have uncommitted work in their own directory. A thread whose
   * work could not be checked is left out: archiving it refuses then, and says why.
   */
  async uncommitted(threads: readonly Thread[]): Promise<Set<ThreadId>> {
    const found = await Promise.all(
      threads.map((thread) =>
        this.api
          .call("thread.changes", { thread_id: thread.id, scope: { kind: "unsaved" } })
          .then(({ changes }) => (changes.files.length > 0 ? thread.id : null))
          .catch(() => null),
      ),
    );
    return new Set(found.filter((id): id is ThreadId => id !== null));
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
