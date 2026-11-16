import { ArrowUpIcon, CornerDownRightIcon, PencilIcon, SquareIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import type { ProviderInfo, SendMode, Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { contextUsed } from "../../state/stats";
import { type QueuedMessage, type Timeline, queuedMessages } from "../../state/timeline";
import { compactNumber } from "../../util/format";
import { readText, saveText } from "../../util/storage";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "@/components/ui/hover-card";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupTextarea } from "@/components/ui/input-group";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

// Unsent text survives switching between threads.
const drafts = new Map<string, string>();

// What Enter does while the agent works; Alt+Enter does the other. Remembered for
// every thread.
const DELIVERY = "qcc.composer.delivery";

function readDelivery(): SendMode {
  return readText(DELIVERY) === "steer" ? "steer" : "queue";
}

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
  const [preferred, setPreferred] = useState<SendMode>(readDelivery);
  const busy = thread.status === "active";
  const steerable = provider?.capabilities.steer ?? false;
  const canSteer = busy && steerable;
  // While nothing runs a message simply starts a turn.
  const delivery: SendMode = canSteer ? preferred : "queue";
  const other: SendMode = delivery === "steer" ? "queue" : "steer";
  const queued = useMemo(() => queuedMessages(timeline), [timeline]);

  useEffect(() => {
    drafts.set(thread.id, text);
    const element = input.current;
    if (element) {
      element.style.height = "auto";
      element.style.height = `${Math.min(element.scrollHeight, 240)}px`;
    }
  }, [thread.id, text]);

  useEffect(() => input.current?.focus(), []);

  const send = async (mode: SendMode) => {
    const message = text.trim();
    if (!message || sending) return;
    setSending(true);
    const taken = await store().send(thread.id, message, mode);
    setSending(false);
    if (taken) setText("");
    input.current?.focus();
  };

  const choose = (mode: SendMode) => {
    setPreferred(mode);
    saveText(DELIVERY, mode);
  };

  /** Put a queued message back into the composer, after what is typed there. */
  const edit = async (message: QueuedMessage) => {
    const back = await store().unqueue(thread.id, message.queueId);
    if (back === null) return;
    setText((current) => (current.trim() ? `${current.trimEnd()}\n\n${back}` : back));
    input.current?.focus();
  };

  const steerNow = async (message: QueuedMessage) => {
    const back = await store().unqueue(thread.id, message.queueId);
    if (back !== null) await store().send(thread.id, back, "steer");
  };

  if (thread.archived) {
    return (
      <div className="border-t p-3.5 text-center text-sm text-muted-foreground">
        This thread is archived. Its timeline stays readable.
      </div>
    );
  }

  const usage = timeline.usage;
  const used = usage ? contextUsed(usage) : null;
  const status = statusText(thread);
  return (
    <div className="border-t px-6 pt-2.5 pb-3.5">
      <form
        className="mx-auto max-w-[812px]"
        onSubmit={(event) => {
          event.preventDefault();
          void send(delivery);
        }}
      >
        {queued.length > 0 && (
          <QueuedList
            messages={queued}
            canSteer={canSteer}
            steer={(message) => void steerNow(message)}
            edit={(message) => void edit(message)}
            remove={(message) => void store().unqueue(thread.id, message.queueId)}
          />
        )}
        <InputGroup className="bg-card">
          <InputGroupTextarea
            ref={input}
            rows={1}
            className="max-h-60 min-h-0 px-3 pt-2.5"
            value={text}
            placeholder={
              !busy
                ? "Message the agent…"
                : delivery === "steer"
                  ? "Add to what the agent is doing now…"
                  : "Queue a message for when this turn ends…"
            }
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
              event.preventDefault();
              void send(event.altKey && canSteer ? other : delivery);
            }}
          />
          <InputGroupAddon align="block-end" className="flex-wrap gap-x-3 gap-y-1.5">
            <span className={busy ? "status-working" : undefined}>{status}</span>
            {busy && !steerable && (
              <span title="This agent cannot take a message in the middle of a turn, so messages wait for it to end">
                Queue only
              </span>
            )}
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
              <ToggleGroup
                size="sm"
                variant="outline"
                spacing={0}
                aria-label="What Enter does while the agent works"
                value={[delivery]}
                onValueChange={(values: unknown[]) => {
                  const next = values[0];
                  if (next === "queue" || next === "steer") choose(next);
                }}
              >
                <ToggleGroupItem
                  value="queue"
                  className="data-pressed:bg-accent data-pressed:text-foreground"
                  title="Send it as its own turn once this one ends"
                >
                  Queue
                </ToggleGroupItem>
                <ToggleGroupItem
                  value="steer"
                  className="data-pressed:bg-accent data-pressed:text-foreground"
                  title="Feed it into the turn that is running now"
                >
                  Steer
                </ToggleGroupItem>
              </ToggleGroup>
            )}
            <InputGroupButton
              type="submit"
              variant="default"
              size="sm"
              disabled={!text.trim() || sending}
              title={canSteer ? `Enter · Alt+Enter to ${other} instead` : "Enter"}
            >
              {!busy ? "Send" : delivery === "steer" ? "Steer" : "Queue"} <ArrowUpIcon />
            </InputGroupButton>
          </InputGroupAddon>
        </InputGroup>
      </form>
    </div>
  );
}

/** Messages waiting for the running turn, each of which can still be changed. */
function QueuedList(props: {
  messages: readonly QueuedMessage[];
  canSteer: boolean;
  steer: (message: QueuedMessage) => void;
  edit: (message: QueuedMessage) => void;
  remove: (message: QueuedMessage) => void;
}) {
  return (
    <section aria-label="Queued messages" className="mb-2 rounded-lg border bg-card/60 px-3 py-2">
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">
        Queued — each runs as its own turn when the agent is free
      </h3>
      <ol className="flex flex-col gap-1">
        {props.messages.map((message) => (
          <li key={message.queueId} className="flex items-center gap-2 text-sm">
            <span className="min-w-0 flex-1 truncate" title={message.text}>
              {message.text}
            </span>
            {props.canSteer && (
              <Button type="button" variant="ghost" size="xs" onClick={() => props.steer(message)} title="Feed it into the running turn now">
                <CornerDownRightIcon /> Steer now
              </Button>
            )}
            <Button type="button" variant="ghost" size="icon-xs" onClick={() => props.edit(message)} aria-label="Edit" title="Take it back into the composer">
              <PencilIcon />
            </Button>
            <Button type="button" variant="ghost" size="icon-xs" onClick={() => props.remove(message)} aria-label="Remove" title="Drop it from the queue">
              <XIcon />
            </Button>
          </li>
        ))}
      </ol>
    </section>
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
