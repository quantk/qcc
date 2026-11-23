// Forking a thread: a new thread that goes on from one of its replies, with the
// conversation up to there, while the thread itself stays as it is. The fork is
// offered under each turn's last reply, and a fork links back to where it came from.

import { GitForkIcon } from "lucide-react";

import type { Thread } from "../../api/protocol.gen";
import { forkBlocker, turnOfReply } from "../../state/fork";
import { providersOf } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { type ItemContext, type UiModule, ui } from "../../ui/registry";
import { Button } from "@/components/ui/button";
import { ForkDialog } from "./ForkDialog";

/** The ended turn whose last reply the item is. */
function turnOf({ item, timeline }: ItemContext) {
  return item.type === "agentMessage" && timeline ? turnOfReply(timeline.turns, item.id) : undefined;
}

/** Why the thread cannot be forked from the item's turn now; null when it can. */
function blocker(context: ItemContext): string | null {
  const turn = turnOf(context);
  if (!turn || !context.timeline) return "Only a reply that ended a turn can be forked from";
  const provider = providersOf(store().getState().allPlugins).find((info) => info.id === turn.provider);
  return forkBlocker(context.timeline.turns, turn, provider && { name: provider.name, capabilities: provider.capabilities });
}

/** The header's way back to the thread a fork came from. */
function ForkedFrom({ thread }: { thread: Thread }) {
  const origin = thread.forked_from;
  const source = useApp((state) => (origin ? state.threads[origin.thread] : undefined));
  if (!origin) return null;
  const name = source?.title ?? "the thread it came from";
  return (
    <Button variant="outline" size="sm" title={`Forked from: ${name}`} onClick={() => store().select(origin.thread)}>
      <GitForkIcon /> {source?.title ?? "Forked from"}
    </Button>
  );
}

export const forkModule: UiModule = {
  id: "core.fork",
  activate(registry) {
    registry.itemMenuItem({
      id: "fork",
      title: "Fork the thread from here…",
      order: 10,
      when: (context) => turnOf(context) !== undefined,
      blocker,
      run: (context) => {
        const turn = turnOf(context);
        if (!turn) return;
        ui.showDialog(({ close }) => <ForkDialog thread={context.thread} turnId={turn.turnId} close={close} />);
      },
    });
    registry.threadAction({
      id: "forked-from",
      order: -9,
      when: ({ thread }) => Boolean(thread.forked_from),
      render: ({ thread }) => <ForkedFrom thread={thread} />,
    });
  },
};
