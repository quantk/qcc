// The Confirmation pattern of AI Elements: a request the agent waits on is an Item with
// its answers as buttons; once answered it becomes one line of the trace.
import { CircleCheckIcon, CircleSlashIcon, CircleXIcon, MessageCircleQuestionIcon, ShieldQuestionIcon } from "lucide-react";
import { useState } from "react";

import type {
  InteractionId,
  InteractionOption,
  InteractionRequest,
  InteractionResolution,
  ThreadId,
} from "../../api/protocol.gen";
import { store } from "../../state/store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from "@/components/ui/item";
import { Trace, TraceDescription, TraceIcon, TraceLine, TraceTitle } from "@/components/ui/trace";

function optionVariant(option: InteractionOption) {
  return option.kind === "allow" ? "default" : option.kind === "deny" ? "destructive" : "outline";
}

function requestTitle(request: InteractionRequest): string {
  return request.type === "approval" ? request.title : request.question;
}

/** An approval or a question the agent waits on, with the controls to answer it. */
export function InteractionPrompt(props: { threadId: ThreadId; id: InteractionId; request: InteractionRequest }) {
  const { threadId, id, request } = props;
  const [text, setText] = useState("");
  const [sent, setSent] = useState(false);
  const options = request.options ?? [];

  const answer = (resolution: InteractionResolution) => {
    setSent(true);
    void store()
      .resolve(threadId, id, resolution)
      .then((result) => {
        if (result === null) setSent(false);
      });
  };

  // The blue border is the overview's color for "needs you".
  return (
    <Item
      variant="outline"
      size="sm"
      className="border-(--blue) bg-card"
      role="group"
      aria-label="The agent is waiting for your answer"
      onClick={(event) => event.stopPropagation()}
    >
      <ItemMedia variant="icon" className="self-start">
        {request.type === "approval" ? <ShieldQuestionIcon /> : <MessageCircleQuestionIcon />}
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="wrap-anywhere">{requestTitle(request)}</ItemTitle>
        {request.type === "approval" && request.detail && (
          <ItemDescription className="line-clamp-none">
            <pre className="max-h-48 overflow-auto font-mono text-xs whitespace-pre-wrap wrap-anywhere">{request.detail}</pre>
          </ItemDescription>
        )}
      </ItemContent>
      {options.length > 0 && (
        <ItemActions className="flex-wrap">
          {options.map((option) => (
            <Button
              key={option.id}
              size="sm"
              variant={optionVariant(option)}
              disabled={sent}
              onClick={() => answer({ type: "selected", option_id: option.id })}
            >
              {option.label}
            </Button>
          ))}
        </ItemActions>
      )}
      {request.type === "question" && request.allow_text && (
        <form
          className="flex basis-full gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (text.trim()) answer({ type: "text", text });
          }}
        >
          <Input value={text} onChange={(event) => setText(event.target.value)} placeholder="Type an answer" />
          <Button size="sm" type="submit" disabled={sent || !text.trim()}>
            Answer
          </Button>
        </form>
      )}
    </Item>
  );
}

/** A request that was answered, or settled without an answer, as a line of the trace. */
export function InteractionOutcome(props: { request: InteractionRequest; resolution: InteractionResolution }) {
  const { request, resolution } = props;
  const option =
    resolution.type === "selected" ? (request.options ?? []).find((candidate) => candidate.id === resolution.option_id) : undefined;
  const answer =
    resolution.type === "selected"
      ? (option?.label ?? resolution.option_id)
      : resolution.type === "text"
        ? resolution.text
        : "not answered";
  const denied = option?.kind === "deny";
  const icon =
    resolution.type === "cancelled" ? <CircleSlashIcon /> : denied ? <CircleXIcon /> : <CircleCheckIcon />;
  return (
    <Trace>
      <TraceLine status={denied ? "error" : undefined}>
        <TraceIcon>{icon}</TraceIcon>
        <TraceTitle title={requestTitle(request)}>{requestTitle(request)}</TraceTitle>
        <TraceDescription className="max-w-1/2 truncate" title={answer}>
          {answer}
        </TraceDescription>
      </TraceLine>
    </Trace>
  );
}
