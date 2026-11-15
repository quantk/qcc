import type { McpServerSettings } from "../../api/protocol.gen";
import { ui } from "../../ui/registry";
import { McpServerDialog } from "./McpServerDialog";

export const MCP_VIEW = "mcp";

/** Change the server called `name`, or add one. */
export function openMcpServer(name?: string, server?: McpServerSettings) {
  ui.showDialog(({ close }) => <McpServerDialog close={close} name={name} server={server} />);
}
