import { describe, expect, it } from "vitest";

import type { LimitWindow, PluginInfo, ProviderLimits } from "../api/protocol.gen";
import {
  age,
  allWindows,
  glance,
  isStale,
  knownLimits,
  limitProviders,
  mainWindows,
  mergeLimits,
  shortProviderName,
  span,
  wantsRefresh,
  windowView,
  worst,
} from "./limits";

const now = 1_800_000_000_000;
const minute = 60_000;

function window(patch: Partial<LimitWindow>): LimitWindow {
  return { id: "five_hour", label: "5-hour", used_percent: 42, window_minutes: 300, ...patch };
}

function limits(windows: LimitWindow[], patch: Partial<ProviderLimits> = {}): ProviderLimits {
  return { provider: "claude-code", plan: "max", windows, updated_at: now - 3 * minute, ...patch };
}

describe("limits", () => {
  it("says spans and ages the way people do", () => {
    expect(span(30_000)).toBe("<1m");
    expect(span(45 * minute)).toBe("45m");
    expect(span(130 * minute)).toBe("2h 10m");
    expect(span((3 * 24 + 4) * 60 * minute)).toBe("3d 4h");
    expect(age(now - 10_000, now)).toBe("just now");
    expect(age(now - 25 * minute, now)).toBe("25 min ago");
    expect(age(now - 3 * 60 * minute, now)).toBe("3 h ago");
  });

  it("shows a window with its reset and how worrying it is", () => {
    const view = windowView(window({ used_percent: 85, resets_at: now + 130 * minute }), now);
    expect(view).toMatchObject({ short: "5h", name: "5-hour", percent: 85, level: "high", reset: "resets in 2h 10m" });
    expect(windowView(window({ used_percent: 97 }), now).level).toBe("full");
    expect(windowView(window({ used_percent: 50, status: "exceeded" }), now).level).toBe("full");
    expect(windowView(window({ used_percent: 10 }), now)).toMatchObject({ level: "ok", reset: null });
    const scoped = windowView(window({ label: "Weekly", window_minutes: 10_080, scope: "Opus" }), now);
    expect(scoped).toMatchObject({ short: "week Opus", name: "Weekly · Opus", scoped: true });
  });

  it("counts a window that started over since the report as empty", () => {
    const over = windowView(window({ used_percent: 100, status: "exceeded", resets_at: now - minute }), now);
    expect(over).toMatchObject({ percent: 0, level: "ok", reset: "reset since the report" });
  });

  it("puts the plan's windows first, shortest first, and the scoped ones after", () => {
    const known = limits([
      window({ id: "seven_day:fable", label: "Weekly", window_minutes: 10_080, scope: "Fable", used_percent: 26 }),
      window({ id: "seven_day", label: "Weekly", window_minutes: 10_080, used_percent: 18 }),
      window({ id: "five_hour", used_percent: 42 }),
    ]);
    expect(mainWindows(known, now).map((w) => w.id)).toEqual(["five_hour", "seven_day"]);
    expect(allWindows(known, now).map((w) => w.id)).toEqual(["five_hour", "seven_day", "seven_day:fable"]);
    expect(glance("Claude", mainWindows(known, now))).toBe("Claude 5h 42% · week 18%");
    expect(glance("Codex", [])).toBe("Codex: no usage limits");
    expect(worst(allWindows(known, now))).toBe("ok");
    expect(worst([windowView(window({ used_percent: 81 }), now), windowView(window({ used_percent: 99 }), now)])).toBe(
      "full",
    );
  });

  it("knows when the news is old", () => {
    expect(isStale(limits([]), now)).toBe(false);
    expect(isStale(limits([], { updated_at: now - 25 * minute }), now)).toBe(true);
  });

  it("lists the providers that report limits and asks again for unknown or old ones", () => {
    const plugins = [
      {
        providers: [
          { id: "claude-code", name: "Claude Code", capabilities: { rate_limits: true } },
          { id: "opencode", name: "opencode", capabilities: { rate_limits: false } },
        ],
      },
      { providers: [{ id: "codex", name: "Codex", capabilities: { rate_limits: true } }] },
    ] as unknown as PluginInfo[];
    const providers = limitProviders(plugins);
    expect(providers).toEqual([
      { id: "claude-code", name: "Claude Code" },
      { id: "codex", name: "Codex" },
    ]);
    expect(shortProviderName("Claude Code")).toBe("Claude");
    const fresh = { "claude-code": limits([]) };
    expect(knownLimits(providers, fresh).map((entry) => entry.id)).toEqual(["claude-code"]);
    expect(wantsRefresh(providers, fresh, now)).toBe(true);
    const both = { ...fresh, codex: limits([], { provider: "codex" }) };
    expect(wantsRefresh(providers, both, now)).toBe(false);
    expect(wantsRefresh(providers, both, now + 10 * minute)).toBe(true);
  });

  it("takes newer limits in and keeps the object when nothing changes", () => {
    const current = { "claude-code": limits([window({})]) };
    expect(mergeLimits(current, [])).toBe(current);
    const older = limits([window({ used_percent: 1 })], { updated_at: now - 60 * minute });
    expect(mergeLimits(current, [older])).toBe(current);
    const newer = limits([window({ used_percent: 50 })], { updated_at: now });
    expect(mergeLimits(current, [newer])["claude-code"]?.windows[0]?.used_percent).toBe(50);
  });
});
