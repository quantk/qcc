import { GitMergeIcon, Undo2Icon } from "lucide-react";
import { useState } from "react";

import type { DeliverMode, Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { baseName } from "../../util/format";
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
import { Button } from "@/components/ui/button";

const explain: Record<DeliverMode, { title: string; text: (source: string) => string; action: string }> = {
  apply: {
    title: "Apply the changes to your checkout?",
    text: (source) =>
      `Everything the thread changed lands in ${source} as uncommitted changes, for you to review and commit. ` +
      "If it does not fit what is there now, nothing is changed.",
    action: "Apply",
  },
  merge: {
    title: "Merge the thread's branch?",
    text: (source) =>
      "What the thread has not committed is committed on its branch, and the branch is merged into the branch " +
      `checked out in ${source}. On a conflict the merge is undone and nothing changes.`,
    action: "Merge",
  },
};

/** Bring a worktree's work back to the checkout it was made from. */
export function DeliverButtons(props: { thread: Thread; disabled: boolean; done: () => void }) {
  const [asking, setAsking] = useState<DeliverMode | null>(null);
  const [working, setWorking] = useState(false);
  const source = props.thread.environment?.source_dir ?? "";
  const run = async (mode: DeliverMode) => {
    setWorking(true);
    const done = await store().deliver(props.thread.id, mode);
    setWorking(false);
    setAsking(null);
    if (done) props.done();
  };
  const ask = asking ? explain[asking] : null;
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        disabled={props.disabled || working}
        title={`Put the changes into ${source}, uncommitted`}
        onClick={() => setAsking("apply")}
      >
        <Undo2Icon /> Apply to {baseName(source)}
      </Button>
      <Button
        variant="outline"
        size="sm"
        disabled={props.disabled || working}
        title={`Commit on the thread's branch and merge it in ${source}`}
        onClick={() => setAsking("merge")}
      >
        <GitMergeIcon /> Merge
      </Button>
      <AlertDialog open={ask !== null} onOpenChange={(open) => !open && setAsking(null)}>
        {ask && asking && (
          <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
            <AlertDialogHeader>
              <AlertDialogTitle>{ask.title}</AlertDialogTitle>
              <AlertDialogDescription>{ask.text(source)}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction disabled={working} onClick={() => void run(asking)}>
                {working ? "Working…" : ask.action}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        )}
      </AlertDialog>
    </>
  );
}
