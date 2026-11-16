// The app's only door to the daemon. The backend passes requests and notifications
// through untouched, so everything here is typed by the generated protocol.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Methods, Notifications, RpcError } from "./protocol.gen";

export type DaemonState =
  | { status: "connecting" }
  | { status: "connected"; socket: string }
  | { status: "disconnected"; error: string };

export type Notification = {
  [N in keyof Notifications]: { method: N; params: Notifications[N] };
}[keyof Notifications];

/** Error codes of the daemon API that the app reacts to. */
export const codes = {
  notConnected: -1,
  environmentError: -32006,
} as const;

export class ApiError extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
  }
}

function toApiError(error: unknown): ApiError {
  if (typeof error === "object" && error !== null && "message" in error) {
    const { code, message } = error as Partial<RpcError>;
    return new ApiError(typeof code === "number" ? code : 0, String(message));
  }
  return new ApiError(0, String(error));
}

export interface Api {
  call<M extends keyof Methods>(
    method: M,
    params: Methods[M]["params"],
  ): Promise<Methods[M]["result"]>;
  /** Both return a function that stops the listening. */
  onNotification(handler: (notification: Notification) => void): () => void;
  onDaemonState(handler: (state: DaemonState) => void): () => void;
  daemonState(): Promise<DaemonState>;
}

function subscribe<T>(event: string, handler: (payload: T) => void): () => void {
  let stopped = false;
  const unlisten = listen<T>(event, ({ payload }) => {
    if (!stopped) handler(payload);
  });
  return () => {
    stopped = true;
    void unlisten.then((stop) => stop());
  };
}

export const tauriApi: Api = {
  async call(method, params) {
    try {
      return await invoke("rpc", { method, params });
    } catch (error) {
      throw toApiError(error);
    }
  },
  onNotification: (handler) => subscribe("daemon-notification", handler),
  onDaemonState: (handler) => subscribe("daemon-state", handler),
  daemonState: () => invoke("daemon_state"),
};
