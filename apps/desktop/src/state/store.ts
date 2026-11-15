// Everything the app knows, in one place. The daemon owns the truth; this store
// mirrors the part of it the window shows and turns user actions into requests.

import { useSyncExternalStore } from "react";

import { type Api, ApiError, type DaemonState, type Notification, codes } from "../api/api";
import type {
  InteractionId,
  InteractionResolution,
  PermissionMode,
  PluginInfo,
  SendMode,
  Thread,
  ThreadEvent,
  ThreadId,
  ThreadSpawnParams,
} from "../api/protocol.gen";
import { type Timeline, applyEvents, emptyTimeline } from "./timeline";

export type AppState = {
  daemon: DaemonState;
  threads: Readonly<Record<ThreadId, Thread>>;
  plugins: readonly PluginInfo[];
  selected: ThreadId | null;
  /** The app view shown while no thread is selected. Absent: the first one. */
  view: string | null;
  timelines: Readonly<Record<ThreadId, Timeline>>;
  /** The selected thread's events are being read. */
  loading: boolean;
  /** The last thing that went wrong, until dismissed. */
  error: string | null;
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
    selected: null,
    view: null,
    timelines: {},
    loading: false,
    error: null,
  };
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
    const stops = [
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
      // Subscriptions die with the connection.
      this.watch = null;
      this.live = null;
    }
  }

  private async connected() {
    const watch = this.nextSubscription++;
    this.watch = watch;
    try {
      const [list, plugins] = await Promise.all([
        this.api.call("thread.watch", { subscription: watch }),
        this.api.call("plugin.list", {}),
      ]);
      if (this.watch !== watch) return;
      this.mergeThreads(list.threads);
      this.set({ plugins: plugins.plugins });
      if (this.state.selected) await this.follow(this.state.selected);
    } catch (error) {
      this.fail(error);
    }
  }

  private notified(notification: Notification) {
    if (notification.method === "thread.updated") {
      if (notification.params.subscription === this.watch) this.mergeThreads([notification.params.thread]);
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
  }

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

  select = (threadId: ThreadId | null) => {
    if (threadId === this.state.selected) return;
    this.set({ selected: threadId, loading: false });
    if (threadId === null) this.unfollow();
    else if (this.state.daemon.status === "connected") void this.follow(threadId);
  };

  /** Leave the selected thread for one of the app's own screens. */
  openView = (view: string) => {
    this.select(null);
    this.set({ view });
  };

  async spawn(params: ThreadSpawnParams): Promise<Thread | null> {
    const result = await this.attempt(() => this.api.call("thread.spawn", params));
    if (!result) return null;
    this.mergeThreads([result.thread]);
    this.select(result.thread.id);
    return result.thread;
  }

  /** Resolves to whether the daemon took the message. */
  async send(threadId: ThreadId, text: string, delivery: SendMode): Promise<boolean> {
    const result = await this.attempt(() =>
      this.api.call("thread.send", { thread_id: threadId, text, delivery }),
    );
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
