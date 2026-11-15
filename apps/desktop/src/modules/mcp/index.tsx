// MCP servers: a screen to set up the ones qcc gives every agent and to see how
// they and the agents' own servers are doing, and a tab with the servers of one
// thread's session.

import { store } from "../../state/store";
import type { UiModule } from "../../ui/registry";
import { McpView } from "./McpView";
import { MCP_VIEW, openMcpServer } from "./open";
import { ThreadMcp } from "./ThreadMcp";

export const mcpModule: UiModule = {
  id: "core.mcp",
  activate(registry) {
    const connected = () => store().getState().daemon.status === "connected";
    registry.appView({ id: MCP_VIEW, title: "MCP servers", order: 90, render: McpView });
    registry.threadTab({
      id: "mcp",
      title: "MCP",
      order: 30,
      when: ({ thread }) => (thread.mcp_servers ?? []).length > 0,
      render: ({ thread }) => <ThreadMcp thread={thread} />,
    });
    registry.command({ id: "mcp.add", title: "Add an MCP server", enabled: connected, run: () => openMcpServer() });
    registry.command({ id: "mcp.open", title: "MCP servers", run: () => store().openView(MCP_VIEW) });
  },
};
