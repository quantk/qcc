import { useEffect, useRef, useState } from "react";

import type { ProviderInfo, SendMode, Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import type { Timeline } from "../../state/timeline";
import { compactNumber, plural } from "../../util/format";

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
    return <div className="composer archived">This thread is archived. Its timeline stays readable.</div>;
  }

  const usage = timeline.usage;
  const queued = thread.queued_inputs ?? 0;
  return (
    <div className="composer">
      <div className="composer-status">
        <span className={busy ? "status-working" : ""}>{statusText(thread)}</span>
        {queued > 0 && <span>{plural(queued, "message")} queued</span>}
        <span className="spacer" />
        {usage && (
          <span title="Tokens in the last request, and the model's context window">
            {compactNumber(usage.last.input_tokens + (usage.last.cached_input_tokens ?? 0))}
            {usage.model_context_window ? ` / ${compactNumber(usage.model_context_window)}` : ""} tokens
          </span>
        )}
      </div>
      <div className="composer-box">
        <textarea
          ref={input}
          rows={1}
          value={text}
          placeholder={busy ? "Queue a message for when this turn ends…" : "Message the agent…"}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
            event.preventDefault();
            void send(event.altKey && canSteer ? "steer" : "queue");
          }}
        />
        <div className="composer-actions">
          {busy && (
            <button className="danger" onClick={() => void store().stop(thread.id)} title="Interrupt the running turn">
              Stop
            </button>
          )}
          {canSteer && (
            <button disabled={!text.trim() || sending} onClick={() => void send("steer")} title="Feed into the running turn (Alt+Enter)">
              Steer
            </button>
          )}
          <button className="primary" disabled={!text.trim() || sending} onClick={() => void send("queue")} title="Enter">
            {busy ? "Queue" : "Send"}
          </button>
        </div>
      </div>
    </div>
  );
}
