import { PanelRightIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import type { ThreadId } from "../api/protocol.gen";
import { SIDE_PANEL_WIDTH, THREAD_MIN } from "../state/layout";
import { attentionLabel, attentionOf, childrenOf, findProvider, threadTitle } from "../state/selectors";
import { SIDE_PANEL_BESIDE, store, useApp } from "../state/store";
import { ResizeHandle } from "../ui/kit";
import { BlockContent, BlockMenu, useSlot } from "../ui/blocks";
import { type ThreadContext, blockApplies, useUi } from "../ui/registry";
import { tailPath } from "../util/format";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

/** The frame of one thread. What fills it — tabs and header controls — is contributed. */
export function ThreadView({ threadId }: { threadId: ThreadId }) {
  const threads = useApp((state) => state.threads);
  const thread = threads[threadId];
  const timeline = useApp((state) => state.timelines[threadId]);
  const loading = useApp((state) => state.loading);
  const plugins = useApp((state) => state.allPlugins);
  const allTabs = useSlot("thread.tabs");
  const allActions = useUi((snapshot) => snapshot.actions);
  const allPanels = useSlot("thread.panel");
  const sidePanel = useApp((state) => state.sidePanel);
  const overlay = useApp((state) => state.sideOverlay);
  const beside = useMediaQuery(SIDE_PANEL_BESIDE);
  const panelOpen = beside ? sidePanel.open : overlay;
  const frame = useRef<HTMLDivElement>(null);

  // Over the thread, the panel hides what is under it: Escape puts it away.
  useEffect(() => {
    if (beside || !overlay) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") store().toggleSidePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [beside, overlay]);
  const [chosen, setChosen] = useState<string | null>(null);

  const provider = useMemo(() => (thread ? findProvider(plugins, thread.provider) : undefined), [plugins, thread]);
  const children = useMemo(() => childrenOf(Object.values(threads), threadId), [threads, threadId]);
  if (!thread) return <div className="empty">{loading ? "Loading…" : "This thread is not known to the daemon."}</div>;

  const parent = thread.parent ? threads[thread.parent] : undefined;
  const context: ThreadContext = { thread, timeline, provider, parent, children };
  const tabs = allTabs.filter((tab) => blockApplies(tab, context));
  const actions = allActions.filter((action) => action.when?.(context) ?? true);
  const tab = tabs.find((candidate) => candidate.id === chosen) ?? tabs[0];
  const panels = allPanels.filter((panel) => blockApplies(panel, context));
  const panel = panels.find((candidate) => candidate.id === sidePanel.tab) ?? panels[0];
  const attention = attentionOf(thread);

  return (
    <div ref={frame} className="relative flex min-h-0 flex-1">
      <div className="thread min-w-0">
        <header className="thread-head">
          <div className="thread-head-main">
            <h1>
              <span className={`dot ${attention}`} title={attentionLabel[attention]} />
              {threadTitle(thread)}
            </h1>
            <div className="thread-facts">
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
            {panel && (
              <Button
                variant={panelOpen ? "secondary" : "ghost"}
                size="icon-sm"
                aria-pressed={panelOpen}
                title={panelOpen ? "Hide the side panel (Ctrl+B)" : "Show the side panel (Ctrl+B)"}
                onClick={store().toggleSidePanel}
              >
                <PanelRightIcon />
              </Button>
            )}
          </div>
        </header>
        {tabs.length > 1 && tab && (
          <Tabs value={tab.id} onValueChange={(value) => setChosen(String(value))} className="border-b px-4">
            <TabsList variant="line" className="h-10">
              {tabs.map((candidate) => (
                <BlockMenu
                  key={candidate.id}
                  block={candidate}
                  render={<TabsTrigger value={candidate.id} className="flex-none px-2.5 after:bg-primary" />}
                >
                  {candidate.title}
                </BlockMenu>
              ))}
            </TabsList>
          </Tabs>
        )}
        {tab ? (
          // A block that scrolls by itself fills this; one that does not scrolls in it.
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            <BlockContent block={tab} context={context} />
          </div>
        ) : (
          <div className="empty">Nothing to show for this thread.</div>
        )}
      </div>
      {panel && panelOpen && (
        <aside
          className={`flex min-h-0 flex-none flex-col border-l bg-muted ${beside ? "relative" : "absolute inset-y-0 right-0 z-20 shadow-xl"}`}
          // Beside the thread, the panel leaves it room; over it, it leaves a strip to see what is under.
          style={{
            width: sidePanel.width,
            maxWidth: beside ? `calc(100% - ${THREAD_MIN}px)` : "calc(100% - 48px)",
          }}
          aria-label="Side panel"
        >
          <ResizeHandle
            edge="left"
            width={sidePanel.width}
            bounds={SIDE_PANEL_WIDTH}
            room={() => (frame.current?.clientWidth ?? window.innerWidth) - (beside ? THREAD_MIN : 48)}
            resize={store().setSidePanelWidth}
            label="Side panel width"
          />
          <div className="flex items-center gap-1 border-b pr-2 pl-3">
            <Tabs value={panel.id} onValueChange={(value) => store().setSidePanel({ tab: String(value) })} className="min-w-0 flex-1">
              {/* The user may put many blocks here: a narrow panel wraps its tabs onto more lines
                  rather than hiding some of them. */}
              <TabsList variant="line" className="h-auto! min-h-10 max-w-full flex-wrap justify-start! gap-y-0 py-1">
                {panels.map((candidate) => (
                  <BlockMenu
                    key={candidate.id}
                    block={candidate}
                    render={<TabsTrigger value={candidate.id} className="h-8 flex-none px-2 after:bg-primary" />}
                  >
                    {candidate.title}
                  </BlockMenu>
                ))}
              </TabsList>
            </Tabs>
            <Button variant="ghost" size="icon-xs" title="Hide the side panel (Ctrl+B)" onClick={store().toggleSidePanel}>
              <XIcon />
            </Button>
          </div>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            <BlockContent block={panel} context={context} />
          </div>
        </aside>
      )}
    </div>
  );
}

function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (changed) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", changed);
      return () => list.removeEventListener("change", changed);
    },
    () => window.matchMedia(query).matches,
  );
}
