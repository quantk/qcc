import { describe, expect, it } from "vitest";

import { DEFAULT_PROJECT_ICON, PROJECT_ICONS, lucideIconOf, projectIconLabel } from "../ui/kit/ProjectIcon";
import { EMOJI_MAX, emojiProblem, pickColor, pickEmoji, pickName, sameIcon } from "./projectIcon";

describe("project icons", () => {
  it("offers only names the daemon takes, and falls back for names it does not know", () => {
    const names = Object.keys(PROJECT_ICONS);
    expect(names.length).toBeGreaterThanOrEqual(40);
    for (const name of names) expect(name).toMatch(/^[a-z0-9]+(-[a-z0-9]+)*$/);
    expect(lucideIconOf("rocket")).toBe(PROJECT_ICONS.rocket);
    expect(lucideIconOf("no-such-icon")).toBe(PROJECT_ICONS[DEFAULT_PROJECT_ICON]);
    // Not fooled by what every object has.
    expect(lucideIconOf("constructor")).toBe(PROJECT_ICONS[DEFAULT_PROJECT_ICON]);
  });

  it("checks an emoji the way the daemon does", () => {
    expect(emojiProblem(" 🚀 ")).toBeNull();
    expect(emojiProblem("🇺🇦")).toBeNull();
    expect(emojiProblem("👩🏻‍❤️‍💋‍👨🏼")).toBeNull();
    expect(emojiProblem("")).not.toBeNull();
    expect(emojiProblem("🚀 🚀")).not.toBeNull();
    expect(emojiProblem("🚀".repeat(EMOJI_MAX + 1))).not.toBeNull();
    expect(pickEmoji(" 🛒")).toEqual({ kind: "emoji", emoji: "🛒" });
    expect(pickEmoji("a b")).toBeUndefined();
  });

  it("keeps the color across names, and the default icon in no color is no icon", () => {
    const teal = pickColor(null, "teal");
    expect(teal).toEqual({ kind: "lucide", name: DEFAULT_PROJECT_ICON, color: "teal" });
    expect(pickName(teal, "rocket")).toEqual({ kind: "lucide", name: "rocket", color: "teal" });
    expect(pickColor(teal, null)).toBeNull();
    expect(pickName(null, DEFAULT_PROJECT_ICON)).toBeNull();
    expect(pickName(null, "bot")).toEqual({ kind: "lucide", name: "bot" });
    const emoji = { kind: "emoji", emoji: "🚀" } as const;
    expect(pickColor(emoji, "red")).toBe(emoji);
    expect(pickName(emoji, "bot")).toEqual({ kind: "lucide", name: "bot" });
  });

  it("compares and names icons", () => {
    expect(sameIcon(null, undefined)).toBe(true);
    expect(sameIcon({ kind: "lucide", name: "bot" }, { kind: "lucide", name: "bot", color: null })).toBe(true);
    expect(sameIcon({ kind: "lucide", name: "bot" }, { kind: "lucide", name: "bot", color: "red" })).toBe(false);
    expect(sameIcon({ kind: "emoji", emoji: "🚀" }, null)).toBe(false);
    expect(projectIconLabel({ kind: "lucide", name: "bot", color: "brand" })).toBe("bot, amber");
    expect(projectIconLabel(null)).toBe("Default icon");
  });
});
