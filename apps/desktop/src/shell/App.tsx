import { useEffect, useState } from "react";

import type { DaemonState } from "../api/api";
import { store, useApp } from "../state/store";
import { matchesShortcut, ui, useUi } from "../ui/registry";
import { CommandPalette } from "./CommandPalette";
import { Sidebar } from "./Sidebar";
import { ThreadView } from "./ThreadView";
import { Button } from "@/components/ui/button";

const PALETTE_SHORTCUT = "Ctrl+K";

function ConnectionBanner({ daemon }: { daemon: DaemonState }) {
  if (daemon.status === "connected") return null;
  return (
    <div className={`banner ${daemon.status}`} role="status">
      {daemon.status === "connecting" ? (
        <>Connecting to qccd…</>
      ) : (
        <>
          <strong>Not connected to qccd.</strong> {daemon.error}. Retrying…
        </>
      )}
    </div>
  );
}

/** The frame: sidebar, the selected thread, and whatever dialog a module opened. */
export function App() {
  const daemon = useApp((state) => state.daemon);
  const selected = useApp((state) => state.selected);
  const error = useApp((state) => state.error);
  const notice = useApp((state) => state.notice);
  const commands = useUi((snapshot) => snapshot.commands);
  const Dialog = useUi((snapshot) => snapshot.dialog);
  const views = useUi((snapshot) => snapshot.views);
  const viewId = useApp((state) => state.view);
  const View = (views.find((view) => view.id === viewId) ?? views[0])?.render;
  const [palette, setPalette] = useState(false);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (matchesShortcut(event, PALETTE_SHORTCUT)) {
        event.preventDefault();
        setPalette(true);
        return;
      }
      const command = commands.find((candidate) => candidate.shortcut && matchesShortcut(event, candidate.shortcut));
      if (!command) return;
      event.preventDefault();
      if (command.enabled?.() ?? true) command.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [commands]);

  // A notice says what just happened; it does not stay.
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(store().dismissNotice, 8000);
    return () => clearTimeout(timer);
  }, [notice]);

  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <ConnectionBanner daemon={daemon} />
        {selected ? (
          <ThreadView key={selected} threadId={selected} />
        ) : View ? (
          <View />
        ) : (
          <div className="empty">Pick a thread on the left.</div>
        )}
      </main>
      {Dialog && <Dialog close={ui.closeDialog} />}
      {palette && <CommandPalette commands={commands} close={() => setPalette(false)} />}
      {notice && !error && (
        <div className="toast notice" role="status">
          <span>{notice}</span>
          <Button variant="outline" size="sm" onClick={store().dismissNotice}>
            Dismiss
          </Button>
        </div>
      )}
      {error && (
        <div className="toast" role="alert">
          <span>{error}</span>
          <Button variant="outline" size="sm" onClick={store().dismissError}>
            Dismiss
          </Button>
        </div>
      )}
    </div>
  );
}
