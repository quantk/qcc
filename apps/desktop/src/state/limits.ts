// The usage limits of the agents' subscriptions, as the daemon keeps them per
// provider: what to show at a glance, how worrying it is and how old the news is.

import type { LimitWindow, PluginInfo, ProviderId, ProviderLimits } from "../api/protocol.gen";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Older than this, limits are marked as such: the agents may have spent more since. */
export const STALE_AFTER = 15 * MINUTE;
/** Older than this, the app asks the agents again when it connects or a view opens. */
export const REFRESH_AFTER = 5 * MINUTE;

/** How worrying a window is: `full` is reached or about to be, `high` past 80%. */
export type LimitLevel = "ok" | "high" | "full";

/** One window as it stands now. */
export type WindowView = {
  id: string;
  /** Short, for a glance: `5h`, `week`. */
  short: string;
  /** For the details: `5-hour`, `Weekly · Opus`. */
  name: string;
  /** Spent now: 0 once the window started over since the report. */
  percent: number;
  level: LimitLevel;
  /** `resets in 2h 10m`, `reset since the report`, or nothing when unknown. */
  reset: string | null;
  /** The window is limited to one model or product. */
  scoped: boolean;
};

/** A span of time the way people say it: `45m`, `2h 10m`, `3d 4h`. */
export function span(ms: number): string {
  const minutes = Math.max(0, Math.floor(ms / MINUTE));
  if (minutes < 1) return "<1m";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  return `${Math.floor(minutes / (24 * 60))}d ${Math.floor((minutes % (24 * 60)) / 60)}h`;
}

/** How old a report is, in words: `just now`, `25 min ago`, `3 h ago`. */
export function age(updatedAt: number, now: number): string {
  const ms = Math.max(0, now - updatedAt);
  if (ms < MINUTE) return "just now";
  if (ms < HOUR) return `${Math.floor(ms / MINUTE)} min ago`;
  if (ms < DAY) return `${Math.floor(ms / HOUR)} h ago`;
  return `${Math.floor(ms / DAY)} d ago`;
}

export function isStale(limits: ProviderLimits, now: number): boolean {
  return now - limits.updated_at > STALE_AFTER;
}

/** A window's short name, by its length where it is a familiar one. */
export function shortName(window: LimitWindow): string {
  switch (window.window_minutes) {
    case 300:
      return "5h";
    case 1440:
      return "day";
    case 10_080:
      return "week";
    default:
      return window.label;
  }
}

export function levelOf(percent: number, exceeded: boolean): LimitLevel {
  if (exceeded || percent >= 95) return "full";
  if (percent >= 80) return "high";
  return "ok";
}

export function windowView(window: LimitWindow, now: number): WindowView {
  const over = window.resets_at != null && window.resets_at <= now;
  // A window that started over since the report has nothing spent that we know of.
  const percent = over ? 0 : window.used_percent;
  const exceeded = !over && window.status === "exceeded";
  let reset: string | null = null;
  if (window.resets_at != null) reset = over ? "reset since the report" : `resets in ${span(window.resets_at - now)}`;
  return {
    id: window.id,
    short: window.scope ? `${shortName(window)} ${window.scope}` : shortName(window),
    name: window.scope ? `${window.label} · ${window.scope}` : window.label,
    percent,
    level: levelOf(percent, exceeded),
    reset,
    scoped: Boolean(window.scope),
  };
}

/** The windows of the account's whole plan, shortest first: what a glance shows. */
export function mainWindows(limits: ProviderLimits, now: number): WindowView[] {
  return limits.windows
    .filter((window) => !window.scope)
    .sort((a, b) => (a.window_minutes ?? Infinity) - (b.window_minutes ?? Infinity))
    .map((window) => windowView(window, now));
}

/** Every window: the main ones shortest first, then the scoped ones in the agent's order. */
export function allWindows(limits: ProviderLimits, now: number): WindowView[] {
  const scoped = limits.windows.filter((window) => window.scope).map((window) => windowView(window, now));
  return [...mainWindows(limits, now), ...scoped];
}

const rank: Record<LimitLevel, number> = { ok: 0, high: 1, full: 2 };

/** The most worrying level among windows. */
export function worst(windows: readonly WindowView[]): LimitLevel {
  return windows.reduce<LimitLevel>((level, window) => (rank[window.level] > rank[level] ? window.level : level), "ok");
}

/** `Claude 5h 42% · week 18%`, for a tooltip or a line. */
export function glance(name: string, windows: readonly WindowView[]): string {
  if (windows.length === 0) return `${name}: no usage limits`;
  return `${name} ${windows.map((window) => `${window.short} ${window.percent}%`).join(" · ")}`;
}

/** The providers that report limits, by id, with the name they go by. */
export function limitProviders(plugins: readonly PluginInfo[]): { id: ProviderId; name: string }[] {
  return plugins.flatMap((plugin) =>
    plugin.providers.filter((provider) => provider.capabilities.rate_limits).map(({ id, name }) => ({ id, name })),
  );
}

/** The name a provider goes by, shortened for a narrow row (`Claude Code` → `Claude`). */
export function shortProviderName(name: string): string {
  return name.replace(/ Code$/, "");
}

/** What is known, in the order of the providers that report limits; a provider never heard from is left out. */
export function knownLimits(
  providers: readonly { id: ProviderId; name: string }[],
  limits: Readonly<Record<ProviderId, ProviderLimits>>,
): { id: ProviderId; name: string; limits: ProviderLimits }[] {
  return providers.flatMap((provider) => {
    const known = limits[provider.id];
    return known ? [{ ...provider, limits: known }] : [];
  });
}

/** Whether the app should ask the agents again: something is unknown or old. */
export function wantsRefresh(
  providers: readonly { id: ProviderId }[],
  limits: Readonly<Record<ProviderId, ProviderLimits>>,
  now: number,
): boolean {
  return providers.some((provider) => {
    const known = limits[provider.id];
    return !known || now - known.updated_at > REFRESH_AFTER;
  });
}

/** Take newer limits in, keeping the object when nothing changed. */
export function mergeLimits(
  current: Readonly<Record<ProviderId, ProviderLimits>>,
  incoming: readonly ProviderLimits[],
): Readonly<Record<ProviderId, ProviderLimits>> {
  let next: Record<ProviderId, ProviderLimits> | null = null;
  for (const limits of incoming) {
    const known = current[limits.provider];
    if (known && known.updated_at > limits.updated_at) continue;
    next ??= { ...current };
    next[limits.provider] = limits;
  }
  return next ?? current;
}
