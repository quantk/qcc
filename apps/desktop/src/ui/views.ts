// How the app's views are laid out in the sidebar: the order of their buttons and
// the commands that open them, shared by the sidebar and the navigation module.

import type { AppView } from "./registry";

/** The buttons in the order they are drawn: the top row, then the foot. */
export function viewButtons(views: readonly AppView[]): { top: AppView[]; bottom: AppView[] } {
  return {
    top: views.filter((view) => view.place !== "bottom"),
    bottom: views.filter((view) => view.place === "bottom"),
  };
}

/** The id of the command that opens a view by its position (`Alt+1`, `Alt+2`, …). */
export function openViewCommand(viewId: string): string {
  return `view.open:${viewId}`;
}

/** The views reachable by `Alt+<digit>`, with their shortcuts, in the order drawn. */
export function viewShortcuts(views: readonly AppView[]): { view: AppView; shortcut: string }[] {
  const { top, bottom } = viewButtons(views);
  return [...top, ...bottom].slice(0, 9).map((view, index) => ({ view, shortcut: `Alt+${index + 1}` }));
}
