import { CircleAlertIcon, CircleCheckIcon, KeyRoundIcon, LoaderCircleIcon } from "lucide-react";

import type { McpState } from "../../api/protocol.gen";
import { mcpStateLabel } from "../../state/mcp";

const icons: Record<McpState, React.ReactNode> = {
  starting: <LoaderCircleIcon className="size-3.5 animate-spin text-muted-foreground" />,
  ready: <CircleCheckIcon className="size-3.5 text-(--green)" />,
  failed: <CircleAlertIcon className="size-3.5 text-destructive" />,
  "needs-auth": <KeyRoundIcon className="size-3.5 text-(--blue)" />,
};

/** How a server is doing, as an icon and, unless `bare`, a word. */
export function McpStateMark({ state, bare, suffix }: { state: McpState; bare?: boolean; suffix?: string }) {
  return (
    <span className="inline-flex items-center gap-1 text-xs whitespace-nowrap" title={mcpStateLabel[state]}>
      {icons[state]}
      {!bare && (
        <span>
          {mcpStateLabel[state]}
          {suffix}
        </span>
      )}
    </span>
  );
}
