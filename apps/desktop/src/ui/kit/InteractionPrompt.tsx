import { useState } from "react";

import type {
  InteractionId,
  InteractionOption,
  InteractionRequest,
  InteractionResolution,
  ThreadId,
} from "../../api/protocol.gen";
import { store } from "../../state/store";

function optionClass(option: InteractionOption): string {
  return option.kind === "allow" ? "primary" : option.kind === "deny" ? "danger" : "";
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

  return (
    <div
      className="interaction pending"
      role="group"
      aria-label="The agent is waiting for your answer"
      onClick={(event) => event.stopPropagation()}
    >
      <div className="interaction-title">{request.type === "approval" ? request.title : request.question}</div>
      {request.type === "approval" && request.detail && <pre className="output">{request.detail}</pre>}
      <div className="interaction-actions">
        {options.map((option) => (
          <button
            key={option.id}
            className={optionClass(option)}
            disabled={sent}
            onClick={() => answer({ type: "selected", option_id: option.id })}
          >
            {option.label}
          </button>
        ))}
      </div>
      {request.type === "question" && request.allow_text && (
        <form
          className="interaction-text"
          onSubmit={(event) => {
            event.preventDefault();
            if (text.trim()) answer({ type: "text", text });
          }}
        >
          <input value={text} onChange={(event) => setText(event.target.value)} placeholder="Type an answer" />
          <button className="primary" disabled={sent || !text.trim()}>
            Answer
          </button>
        </form>
      )}
    </div>
  );
}
