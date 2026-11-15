import { useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { environmentsOf } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

/** Move a thread that works in its directory into a worktree of its own. */
export function IsolateDialog({ thread, close }: { thread: Thread; close: () => void }) {
  const plugins = useApp((state) => state.allPlugins);
  const environment = environmentsOf(plugins)[0];
  const [carry, setCarry] = useState(true);
  const [working, setWorking] = useState(false);
  const move = async () => {
    if (!environment) return;
    setWorking(true);
    const done = await store().isolate(thread.id, environment.id, carry);
    setWorking(false);
    if (done) close();
  };
  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Move into a worktree</DialogTitle>
          <DialogDescription>
            The thread goes on in a git worktree of its own, on a new branch from the current commit of {thread.cwd}.
            Its agent starts a new session there and is told the conversation so far. Your checkout is free again.
          </DialogDescription>
        </DialogHeader>
        <label className="flex items-start gap-2.5 text-sm">
          <Checkbox checked={carry} onCheckedChange={(checked) => setCarry(checked)} className="mt-0.5" />
          <span>
            Take the uncommitted changes along
            <span className="block text-[13px] text-muted-foreground">
              All of them, new files included, also any you made yourself: they leave this directory and continue in
              the worktree. Unchecked, the worktree starts clean and the changes stay here.
            </span>
          </span>
        </label>
        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          <Button disabled={working || !environment} onClick={() => void move()}>
            {working ? "Moving…" : "Move"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
