import { useEffect, useMemo, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { type Stayed, type TreeMember, hasWorkdir, isWorking, liveDescendants, treeWarning } from "../../state/archive";
import { threadTitle } from "../../state/selectors";
import { store } from "../../state/store";
import type { UiRegistry } from "../../ui/registry";
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
export async function archiveThread(ui: Pick<UiRegistry, "showDialog">, thread: Thread): Promise<void> {
  const afterwards = (stayed: Stayed[] | null) => {
    if (stayed) ui.showDialog(({ close }) => <StayedDialog thread={thread} stayed={stayed} close={close} />);
  };
  const members = liveDescendants(Object.values(store().getState().threads), thread.id);
  if (members.length === 0) {
    afterwards(await store().archive(thread.id, false));
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
    props.close();
    props.afterwards(stayed);
  };

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

/** Shown when threads stayed: most often a worktree with uncommitted work. */
function StayedDialog(props: { thread: Thread; stayed: Stayed[]; close: () => void }) {
  const { stayed } = props;
  // Only the thread asked for stayed: say its reason as the old dialog did.
  const only = stayed.length === 1 && stayed[0]?.id === props.thread.id ? stayed[0] : null;
  const one = only !== null;
  return (
    <AlertDialog open onOpenChange={(open) => !open && props.close()}>
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
          <AlertDialogCancel>Keep {one ? "the thread" : "them"}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() =>
              void store()
                .archive(props.thread.id, true)
                .then((still) => {
                  props.close();
                  if (still) store().notify(`${still.length} of the threads could still not be archived`);
                })
            }
          >
            Archive anyway
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
