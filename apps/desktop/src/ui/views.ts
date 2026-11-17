// The app's views as buttons of the sidebar: which blocks can be one, and the
// commands that open them, shared by the sidebar and the navigation module. Which
// views stand at the top and which at the foot is the layout's call (the `views`
// and `views.foot` slots).

import type { AppBlock, Block } from "./registry";

/** Only a block that needs nothing can fill the window as a screen. */
export const isAppBlock = (block: Block): block is AppBlock => block.scope === "app";

/** The id of the command that opens a view by its position (`Alt+1`, `Alt+2`, …). */
export function openViewCommand(viewId: string): string {
  return `view.open:${viewId}`;
}

/** The views reachable by `Alt+<digit>`, with their shortcuts, in the order drawn: the top row, then the foot. */
export function viewShortcuts(top: readonly Block[], bottom: readonly Block[]): { view: AppBlock; shortcut: string }[] {
  return [...top, ...bottom]
    .filter(isAppBlock)
    .slice(0, 9)
    .map((view, index) => ({ view, shortcut: `Alt+${index + 1}` }));
}
