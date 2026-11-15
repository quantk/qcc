import { useMemo, useState } from "react";

import type { ThreadId } from "../api/protocol.gen";
import { attentionLabel, attentionOf, findProvider } from "../state/selectors";
import { useApp } from "../state/store";
import { type ThreadContext, useUi } from "../ui/registry";
import { tailPath } from "../util/format";

/** The frame of one thread. What fills it — tabs and header controls — is contributed. */
export function ThreadView({ threadId }: { threadId: ThreadId }) {
  const thread = useApp((state) => state.threads[threadId]);
  const timeline = useApp((state) => state.timelines[threadId]);
  const loading = useApp((state) => state.loading);
  const plugins = useApp((state) => state.plugins);
  const allTabs = useUi((snapshot) => snapshot.tabs);
  const allActions = useUi((snapshot) => snapshot.actions);
  const [chosen, setChosen] = useState<string | null>(null);

  const provider = useMemo(() => (thread ? findProvider(plugins, thread.provider) : undefined), [plugins, thread]);
  if (!thread) return <div className="empty">{loading ? "Loading…" : "This thread is not known to the daemon."}</div>;

  const context: ThreadContext = { thread, timeline, provider };
  const tabs = allTabs.filter((tab) => tab.when?.(context) ?? true);
  const actions = allActions.filter((action) => action.when?.(context) ?? true);
  const tab = tabs.find((candidate) => candidate.id === chosen) ?? tabs[0];
  const attention = attentionOf(thread);

  return (
    <div className="thread">
      <header className="thread-head">
        <div className="thread-head-main">
          <h1>
            <span className={`dot ${attention}`} title={attentionLabel[attention]} />
            {thread.title ?? "New thread"}
          </h1>
          <div className="thread-facts">
            <span>{provider?.name ?? thread.provider}</span>
            {thread.model && <span>{thread.model}</span>}
            <span className="path" title={thread.cwd}>
              {tailPath(thread.cwd, 70)}
            </span>
            {thread.environment && (
              <span title={`Made from ${thread.environment.source_dir}`}>
                {thread.environment.id} · {thread.environment.state}
              </span>
            )}
          </div>
        </div>
        <div className="thread-head-actions">
          {actions.map((action) => (
            <action.render key={action.id} {...context} />
          ))}
        </div>
      </header>
      {tabs.length > 1 && (
        <div className="tabs" role="tablist">
          {tabs.map((candidate) => (
            <button key={candidate.id} role="tab" aria-selected={candidate === tab} onClick={() => setChosen(candidate.id)}>
              {candidate.title}
            </button>
          ))}
        </div>
      )}
      {tab ? <tab.render {...context} /> : <div className="empty">Nothing to show for this thread.</div>}
    </div>
  );
}
