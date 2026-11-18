import type { Automation, AutomationId } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { ui } from "../../ui/registry";
import { AutomationDialog } from "./AutomationDialog";

export const AUTOMATIONS_VIEW = "automations";

/** Change the automation given, or make one. */
export function openAutomation(automation?: Automation) {
  ui.showDialog(({ close }) => <AutomationDialog close={close} automation={automation} />);
}

/** The automation the screen opens with its runs shown, once; see `showAutomation`. */
let focused: AutomationId | null = null;

/** Open the Automations screen with one automation's runs unfolded. */
export function showAutomation(id: AutomationId) {
  focused = id;
  store().openView(AUTOMATIONS_VIEW);
}

/** Whether the screen was opened for this automation; asking forgets it. */
export function takeFocus(id: AutomationId): boolean {
  if (focused !== id) return false;
  focused = null;
  return true;
}
