// Threads that start threads. An agent that delegates through qcc becomes the
// parent of what it starts; this module links the two ways: up to the parent from
// the header, down to the children in a tab of their own.

import type { UiModule } from "../../ui/registry";
import { ChildrenView, ParentLink } from "./LineageViews";

export const lineageModule: UiModule = {
  id: "core.lineage",
  activate(ui) {
    ui.threadAction({
      id: "parent",
      order: -10,
      when: ({ parent }) => parent !== undefined,
      render: ({ parent }) => (parent ? <ParentLink parent={parent} /> : null),
    });
    ui.threadTab({
      id: "agents",
      title: "Agents",
      order: 5,
      when: ({ children }) => children.length > 0,
      render: ({ children }) => <ChildrenView children={children} />,
    });
  },
};
