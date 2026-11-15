import { ArrowUpIcon, SquareIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { ProviderInfo, SendMode, Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import type { Timeline } from "../../state/timeline";
import { compactNumber, plural } from "../../util/format";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "@/components/ui/hover-card";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupTextarea } from "@/components/ui/input-group";
import { Progress } from "@/components/ui/progress";

// Unsent text survives switching between threads.
const drafts = new Map<string, string>();

function statusText(thread: Thread): string {
  if ((thread.pending_interactions ?? []).length > 0) return "Waiting for your answer";
  if (thread.status === "active") return "Working…";
  if (thread.status === "failed") return "Failed — send a message to try again";
  if (thread.environment?.state === "pending") return "Preparing the environment…";
  return "";
}

export function Composer(props: { thread: Thread; timeline: Timeline; provider?: ProviderInfo }) {
  const { thread, timeline, provider } = props;
  const [text, setText] = useState(() => drafts.get(thread.id) ?? "");
  const [sending, setSending] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const busy = thread.status === "active";
  const canSteer = busy && (provider?.capabilities.steer ?? false);

  useEffect(() => {
    drafts.set(thread.id, text);
    const element = input.current;
    if (element) {
      element.style.height = "auto";
      element.style.height = `${Math.min(element.scrollHeight, 240)}px`;
    }
  }, [thread.id, text]);

  useEffect(() => input.current?.focus(), []);

  const send = async (delivery: SendMode) => {
    const message = text.trim();
    if (!message || sending) return;
    setSending(true);
    const taken = await store().send(thread.id, message, delivery);
    setSending(false);
    if (taken) setText("");
    input.current?.focus();
  };

  if (thread.archived) {
    return (
      <div className="border-t p-3.5 text-center text-sm text-muted-foreground">
        This thread is archived. Its timeline stays readable.
      </div>
    );
  }

  const usage = timeline.usage;
  const used = usage ? usage.last.input_tokens + (usage.last.cached_input_tokens ?? 0) : null;
  const queued = thread.queued_inputs ?? 0;
  const status = statusText(thread);
  return (
    <div className="border-t px-6 pt-2.5 pb-3.5">
      <form
        className="mx-auto max-w-[812px]"
        onSubmit={(event) => {
          event.preventDefault();
          void send("queue");
        }}
      >
        <InputGroup className="bg-card">
          <InputGroupTextarea
            ref={input}
            rows={1}
            className="max-h-60 min-h-0 px-3 pt-2.5"
            value={text}
            placeholder={busy ? "Queue a message for when this turn ends…" : "Message the agent…"}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
              event.preventDefault();
              void send(event.altKey && canSteer ? "steer" : "queue");
            }}
          />
          <InputGroupAddon align="block-end" className="gap-3">
            <span className={busy ? "status-working" : undefined}>{status}</span>
            {queued > 0 && <span>{plural(queued, "message")} queued</span>}
            <span className="ml-auto" />
            {used !== null && <ContextUsage used={used} limit={usage?.model_context_window ?? null} />}
            {busy && (
              <InputGroupButton
                variant="destructive"
                size="sm"
                onClick={() => void store().stop(thread.id)}
                title="Interrupt the running turn"
              >
                <SquareIcon /> Stop
              </InputGroupButton>
            )}
            {canSteer && (
              <InputGroupButton
                variant="outline"
                size="sm"
                disabled={!text.trim() || sending}
                onClick={() => void send("steer")}
                title="Feed into the running turn (Alt+Enter)"
              >
                Steer
              </InputGroupButton>
            )}
            <InputGroupButton type="submit" variant="default" size="sm" disabled={!text.trim() || sending} title="Enter">
              {busy ? "Queue" : "Send"} <ArrowUpIcon />
            </InputGroupButton>
          </InputGroupAddon>
        </InputGroup>
      </form>
    </div>
  );
}

/** How much of the model's context window the last request used: the Context pattern. */
function ContextUsage({ used, limit }: { used: number; limit: number | null }) {
  const percent = limit ? Math.min(100, Math.round((used / limit) * 100)) : null;
  const label = percent !== null ? `${percent}% context` : `${compactNumber(used)} tokens`;
  if (!limit) return <span title="Tokens in the last request">{label}</span>;
  return (
    <HoverCard>
      <HoverCardTrigger delay={10} closeDelay={100} render={<span className="cursor-default tabular-nums" />}>
        {label}
      </HoverCardTrigger>
      <HoverCardContent className="flex w-64 flex-col gap-3">
        <div className="flex items-center justify-between gap-3 text-sm">
          <span className="font-medium">Context</span>
          <span className="text-muted-foreground tabular-nums">
            {compactNumber(used)} / {compactNumber(limit)}
          </span>
        </div>
        <Progress value={percent} aria-label="Context used" />
      </HoverCardContent>
    </HoverCard>
  );
}
