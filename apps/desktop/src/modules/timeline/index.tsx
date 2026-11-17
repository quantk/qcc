// The conversation: the timeline of a thread, its composer, and how each built-in
// kind of item is drawn.

import type { UiModule } from "../../ui/registry";
import { Composer } from "./Composer";
import { CORE_ITEM_TYPES, CoreItem } from "./items";
import { TimelineView } from "./TimelineView";

export const timelineModule: UiModule = {
  id: "core.timeline",
  activate(ui) {
    ui.block({
      id: "timeline",
      title: "Timeline",
      scope: "thread",
      placement: { slot: "thread.tabs", order: 0 },
      render: (context) =>
        context.timeline ? (
          <>
            <TimelineView thread={context.thread} timeline={context.timeline} />
            <Composer thread={context.thread} timeline={context.timeline} provider={context.provider} context={context} />
          </>
        ) : (
          <div className="empty">Loading…</div>
        ),
    });
    for (const type of CORE_ITEM_TYPES) ui.itemRenderer(type, CoreItem);
  },
};
