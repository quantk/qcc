// The live wall: every agent that is working, waiting or just finished, each with
// what it is doing right now, so they can be watched together.

import { RadioIcon } from "lucide-react";
import { useMemo } from "react";

import { liveCount } from "../../state/live";
import { store, useApp } from "../../state/store";
import type { UiModule, ViewBadge } from "../../ui/registry";
import { plural } from "../../util/format";
import { LiveWall } from "./LiveWall";

export const LIVE = "live";

/** How many agents the wall has at work; tinted when one of them waits for the user. */
function useLiveBadge(): ViewBadge | null {
  const threads = useApp((state) => state.threads);
  return useMemo(() => {
    const { working, waiting } = liveCount(Object.values(threads));
    if (working + waiting === 0) return null;
    const parts = [working > 0 && `${working} working`, waiting > 0 && `${waiting} waiting for you`].filter(Boolean);
    return {
      count: working + waiting,
      tone: waiting > 0 ? "attention" : "default",
      label: `${plural(working + waiting, "agent")}: ${parts.join(", ")}`,
    };
  }, [threads]);
}

export const liveModule: UiModule = {
  id: "core.live",
  activate(ui) {
    ui.block({
      id: LIVE,
      title: "Live",
      scope: "app",
      placement: { slot: "views", order: 5 },
      icon: RadioIcon,
      useBadge: useLiveBadge,
      render: LiveWall,
    });
    ui.command({ id: "view.live", title: "Watch the agents at work", shortcut: "Ctrl+Shift+L", run: () => store().openView(LIVE) });
  },
};
