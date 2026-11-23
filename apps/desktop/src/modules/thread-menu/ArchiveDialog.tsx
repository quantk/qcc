import { useEffect, useMemo, useRef, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import {
  PROGRESS_DELAY_MS,
  type Stayed,
  type TreeMember,
  archiveSummary,
  hasWorkdir,
  isWorking,
  liveDescendants,
  treeWarning,
} from "../../state/archive";
import { threadTitle } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import type { DialogRenderer, UiRegistry } from "../../ui/registry";
import { ArchiveProgressView, useDelayed } from "./ArchiveProgress";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";

/**
 * Archive a thread, from wherever the user asked. With threads under it, the user
 * first sees what goes along; whatever stays afterwards is shown with the reason.
 * Resolves once archiving is done or a dialog has taken over.
 */
export async function archiveThread(
  ui: Pick<UiRegistry, "showDialog" | "closeDialog" | "getSnapshot">,
  thread: Thread,
): Promise<void> {
  const afterwards = (stayed: Stayed[] | null) => {
    if (stayed) ui.showDialog(({ close }) => <StayedDialog thread={thread} stayed={stayed} close={close} />);
  };
  const members = liveDescendants(Object.values(store().getState().threads), thread.id);
  if (members.length === 0) {
    // Most archives of one thread are over at once; a slow one (a large worktree,
    // an agent slow to let go) shows how it goes.
    const progress: DialogRenderer = ({ close }) => <ProgressDialog thread={thread} close={close} />;
    const timer = setTimeout(() => ui.showDialog(progress), PROGRESS_DELAY_MS);
    const stayed = await store().archive(thread.id, false);
    clearTimeout(timer);
    // Only if it is still up: the user may have hidden it and opened another.
    if (ui.getSnapshot().dialog === progress) ui.closeDialog();
    afterwards(stayed);
    return;
  }
  ui.showDialog(({ close }) => <TreeDialog thread={thread} members={members} close={close} afterwards={afterwards} />);
}

/** Asked before archiving a thread that has threads under it. */
function TreeDialog(props: {
  thread: Thread;
  members: TreeMember[];
  close: () => void;
  afterwards: (stayed: Stayed[] | null) => void;
}) {
  const { thread, members } = props;
  const [uncommitted, setUncommitted] = useState<Set<string> | null>(null);
  const [archiving, setArchiving] = useState(false);
  const progress = useApp((state) => state.archives[thread.id]);
  // Past the delay the confirmation turns into the progress, which may be hidden.
  const showing = useDelayed(archiving) && progress !== undefined && progress.length > 0;
  const hidden = useRef(false);
  const hide = () => {
    hidden.current = true;
    props.close();
  };
  useEffect(() => {
    let live = true;
    const withDirs = [thread, ...members.map((member) => member.thread)].filter(hasWorkdir);
    void store()
      .uncommitted(withDirs)
      .then((found) => live && setUncommitted(found));
    return () => {
      live = false;
    };
  }, [thread, members]);
  const warning = useMemo(() => treeWarning(thread, members, uncommitted), [thread, members, uncommitted]);

  const confirm = async () => {
    setArchiving(true);
    const stayed = await store().archive(thread.id, warning.force);
    if (!hidden.current) props.close();
    props.afterwards(stayed);
  };

  if (showing) return <ProgressDialog thread={thread} close={hide} />;

  return (
    <AlertDialog open onOpenChange={(open) => !open && !archiving && props.close()}>
      <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>{warning.title}</AlertDialogTitle>
          <AlertDialogDescription>
            The threads it started go with it, at any depth.
            {warning.working > 0 && " Working threads are stopped."}
            {warning.uncommitted > 0 && " Uncommitted work in their worktrees is discarded."}
            {warning.checking && " Checking the worktrees for uncommitted work…"}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="max-h-64 overflow-y-auto text-sm" aria-label="Threads archived with it">
          {uncommitted?.has(thread.id) && (
            <li className="flex items-center gap-2 py-0.5">
              <span className="truncate">{threadTitle(thread)}</span>
              <Badge variant="destructive">uncommitted changes</Badge>
            </li>
          )}
          {members.map(({ thread: member, depth }) => (
            <li key={member.id} className="flex items-center gap-2 py-0.5" style={{ paddingLeft: `${(depth - 1) * 1}rem` }}>
              <span className="truncate">{threadTitle(member)}</span>
              {isWorking(member) && <Badge variant="secondary">working, will be stopped</Badge>}
              {uncommitted?.has(member.id) && <Badge variant="destructive">uncommitted changes</Badge>}
            </li>
          ))}
        </ul>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={archiving}>Keep them</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={warning.checking || archiving}
            onClick={() => void confirm()}
          >
            {archiving ? "Archiving…" : warning.confirm}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** An archive that takes a while: what it is doing, thread by thread. */
function ProgressDialog(props: { thread: Thread; close: () => void }) {
  const progress = useApp((state) => state.archives[props.thread.id]);
  const threads = progress && progress.length > 0 ? progress : null;
  const title = threads && threads.length > 1 ? archiveSummary(threads).headline : `Archiving "${threadTitle(props.thread)}"`;
  return (
    <AlertDialog open onOpenChange={(open) => !open && props.close()}>
      <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>
            {threads && threads.length > 1
              ? "Deepest first, so no archived thread is left with live threads under it. "
              : ""}
            Archiving goes on if you hide this.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {threads && <ArchiveProgressView threads={threads} />}
        <AlertDialogFooter>
          <AlertDialogCancel>Hide</AlertDialogCancel>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Shown when threads stayed: most often a worktree with uncommitted work. */
function StayedDialog(props: { thread: Thread; stayed: Stayed[]; close: () => void }) {
  const { stayed } = props;
  const [archiving, setArchiving] = useState(false);
  const progress = useApp((state) => state.archives[props.thread.id]);
  const showing = useDelayed(archiving) && progress !== undefined && progress.length > 0;
  const hidden = useRef(false);
  const force = async () => {
    setArchiving(true);
    const still = await store().archive(props.thread.id, true);
    if (!hidden.current) props.close();
    if (still) store().notify(`${still.length} of the threads could still not be archived`);
  };
  if (showing) {
    const hide = () => {
      hidden.current = true;
      props.close();
    };
    return <ProgressDialog thread={props.thread} close={hide} />;
  }
  // Only the thread asked for stayed: say its reason as the old dialog did.
  const only = stayed.length === 1 && stayed[0]?.id === props.thread.id ? stayed[0] : null;
  const one = only !== null;
  return (
    <AlertDialog open onOpenChange={(open) => !open && !archiving && props.close()}>
      <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>Archive and discard the changes?</AlertDialogTitle>
          <AlertDialogDescription>
            {only
              ? `${only.reason}. `
              : `${stayed.length} threads were not archived; the rest were. `}
            Archiving removes a thread's working directory. Work that is not committed there is lost.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {!one && (
          <ul className="max-h-64 overflow-y-auto text-sm" aria-label="Threads not archived">
            {stayed.map((thread) => (
              <li key={thread.id} className="py-0.5">
                <span className="font-medium">{thread.title}</span>
                <span className="text-muted-foreground">: {thread.reason}</span>
              </li>
            ))}
          </ul>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={archiving}>Keep {one ? "the thread" : "them"}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={archiving} onClick={() => void force()}>
            {archiving ? "Archiving…" : "Archive anyway"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
