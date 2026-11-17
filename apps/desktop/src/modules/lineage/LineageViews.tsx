import { ArrowUpIcon } from "lucide-react";

import type { Thread } from "../../api/protocol.gen";
import {
  activityText,
  attentionLabel,
  attentionOf,
  cardTime,
  failureText,
  providerName,
  threadTitle,
} from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { InteractionPrompt } from "../../ui/kit";
import { useNow } from "../../util/useNow";
import { Button } from "@/components/ui/button";

export function ParentLink({ parent }: { parent: Thread }) {
  return (
    <Button
      variant="outline"
      size="sm"
      className="parent-link"
      title={`Started by: ${parent.title ?? parent.id}`}
      onClick={() => store().select(parent.id)}
    >
      <ArrowUpIcon /> {parent.title ?? "Parent thread"}
    </Button>
  );
}

/** The id of the Agents block: the full view of a thread's children. */
export const AGENTS = "agents";

/** What a child is up to, in a line. */
export function childStatus(thread: Thread): string {
  const attention = attentionOf(thread);
  if (attention === "active") return activityText(thread) || "Working";
  if (attention === "waiting") return "Waiting for your answer";
  if (attention === "failed") return failureText(thread);
  if (thread.last_turn_status === "interrupted") return "Stopped";
  return thread.activity?.kind === "agentMessage" && thread.activity.detail ? thread.activity.detail : "Done";
}

/** The threads a thread started: what each is doing, and the questions they ask. */
export function ChildrenView({ children }: { children: readonly Thread[] }) {
  const plugins = useApp((state) => state.allPlugins);
  const now = useNow(1000);
  return (
    <div className="children">
      <p className="hint">Threads this agent started. Their approvals can be answered here.</p>
      {children.map((child) => {
        const attention = attentionOf(child);
        const provider = providerName(plugins, child.provider);
        return (
          <article key={child.id} className={`child-card ${attention}`}>
            <header>
              <span className={`dot ${attention}`} title={attentionLabel[attention]} />
              <Button variant="link" className="h-auto p-0 font-semibold text-foreground" onClick={() => store().select(child.id)}>
                {threadTitle(child)}
              </Button>
              <span className="child-facts">
                {provider}
                {child.environment ? " · worktree" : ""}
              </span>
              <span className="child-time">{cardTime(child, now)}</span>
            </header>
            <div className={`child-status${attention === "active" && child.activity?.kind === "command" ? " technical" : ""}`}>
              {childStatus(child)}
            </div>
            {(child.pending_interactions ?? []).map((pending) => (
              <InteractionPrompt key={pending.id} threadId={child.id} id={pending.id} request={pending.request} />
            ))}
          </article>
        );
      })}
    </div>
  );
}
