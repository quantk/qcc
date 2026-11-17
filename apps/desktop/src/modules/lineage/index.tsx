// Threads that start threads. An agent that delegates through qcc becomes the
// parent of what it starts; this module links the two ways: up to the parent from
// the header, down to the children in a tab of their own, and at a glance beside the
// composer: how many are working, and whether one waits for the user.

import { NetworkIcon } from "lucide-react";

import type { UiModule } from "../../ui/registry";
import { ChildrenChip, ChildrenGlance } from "./ChildrenGlance";
import { AGENTS, ChildrenView, ParentLink } from "./LineageViews";

export const lineageModule: UiModule = {
  id: "core.lineage",
  activate(ui) {
    ui.threadAction({
      id: "parent",
      order: -10,
      when: ({ parent }) => parent !== undefined,
      render: ({ parent }) => (parent ? <ParentLink parent={parent} /> : null),
    });
    ui.block({
      id: AGENTS,
      title: "Agents",
      hint: "The threads this one started",
      scope: "thread",
      placement: { slot: "thread.tabs", order: 5 },
      when: ({ children }) => children.length > 0,
      render: ({ thread }) => <ChildrenView parent={thread} />,
    });
    ui.block({
      id: "agents.glance",
      title: "Started agents",
      hint: "How the threads this one started are doing, at a glance",
      scope: "thread",
      placement: { slot: "thread.composer", order: 0 },
      icon: NetworkIcon,
      when: ({ children }) => children.length > 0,
      summary: ({ children }) => <ChildrenChip children={children} />,
      render: ({ children }) => <ChildrenGlance children={children} />,
    });
  },
};
