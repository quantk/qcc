// What the project icon picker does with a pick. The daemon has the last word on an
// icon; these rules only keep the dialog from sending what it would refuse.

import type { IconColor, ProjectIcon } from "../api/protocol.gen";

/** The icon of a project without one. Kept in sync with `DEFAULT_PROJECT_ICON` of the kit. */
const DEFAULT_NAME = "folder-kanban";

/** The most Unicode scalar values the daemon takes for an emoji (`ICON_EMOJI_MAX`). */
export const EMOJI_MAX = 16;

/** Why `text` cannot be an emoji icon, as the daemon would say; `null` when it can. */
export function emojiProblem(text: string): string | null {
  const emoji = text.trim();
  if (!emoji) return "Type or paste an emoji";
  if ([...emoji].length > EMOJI_MAX) return "Too long for an icon";
  if (/[\s\p{Cc}]/u.test(emoji)) return "One emoji, without spaces";
  return null;
}

/** The default icon in no color is no icon at all: the project keeps none. */
function tidy(icon: ProjectIcon): ProjectIcon | null {
  return icon.kind === "lucide" && icon.name === DEFAULT_NAME && !icon.color ? null : icon;
}

/** A named icon, in the color the icon in use has. */
export function pickName(current: ProjectIcon | null, name: string): ProjectIcon | null {
  const color = current?.kind === "lucide" ? current.color : null;
  return tidy(color ? { kind: "lucide", name, color } : { kind: "lucide", name });
}

/**
 * A color for the icon in use; no icon becomes the default one in that color. An
 * emoji has its own colors and stays as it is.
 */
export function pickColor(current: ProjectIcon | null, color: IconColor | null): ProjectIcon | null {
  if (current?.kind === "emoji") return current;
  const name = current?.name ?? DEFAULT_NAME;
  return tidy(color ? { kind: "lucide", name, color } : { kind: "lucide", name });
}

/** An emoji typed into the picker, or `undefined` while it is not one yet. */
export function pickEmoji(text: string): ProjectIcon | undefined {
  return emojiProblem(text) ? undefined : { kind: "emoji", emoji: text.trim() };
}

export function sameIcon(a: ProjectIcon | null | undefined, b: ProjectIcon | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  if (a.kind === "emoji") return b.kind === "emoji" && a.emoji === b.emoji;
  return b.kind === "lucide" && a.name === b.name && (a.color ?? null) === (b.color ?? null);
}
