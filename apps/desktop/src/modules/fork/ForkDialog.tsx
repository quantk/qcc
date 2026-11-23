import { useState } from "react";

import type { Thread, TurnId } from "../../api/protocol.gen";
import { environmentsOf } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Choice } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

/** Where the fork works: the thread's own directory, or one of its own. */
type Place = "same" | "environment";

/** Fork a thread from the end of one of its turns. */
export function ForkDialog(props: { thread: Thread; turnId: TurnId; close: () => void }) {
  const { thread, turnId, close } = props;
  const plugins = useApp((state) => state.allPlugins);
  const environment = environmentsOf(plugins)[0];
  // Two agents in one directory step on each other's files: a thread with a
  // directory of its own gets a fork with one too.
  const [place, setPlace] = useState<Place>(thread.environment && environment ? "environment" : "same");
  const [prompt, setPrompt] = useState("");
  const [working, setWorking] = useState(false);
  const fork = async () => {
    setWorking(true);
    const forked = await store().fork({
      thread_id: thread.id,
      turn_id: turnId,
      environment: place === "environment" ? environment?.id : null,
      prompt: prompt.trim() || null,
    });
    setWorking(false);
    if (forked) close();
  };
  const options = [
    { value: "same", label: "In the same directory" },
    ...(environment ? [{ value: "environment", label: `In a new ${environment.name.toLowerCase()}` }] : []),
  ];
  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Fork the thread from here</DialogTitle>
          <DialogDescription>
            A new thread goes on from this reply: its agent knows the conversation up to here and nothing after.
            This thread stays as it is. Only the conversation is forked: the files are as they are now.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="fork-place">Where the fork works</Label>
          <Choice
            id="fork-place"
            value={place}
            options={options}
            onChange={(value) => setPlace((value as Place | null) ?? "same")}
          />
          <p className="text-[13px] text-muted-foreground">
            {place === "environment"
              ? `A worktree of its own, on a new branch from the current commit of ${thread.cwd}. What is not committed there yet stays behind: the fork starts without it.`
              : `Right in ${thread.cwd}, beside this thread${thread.status === "active" ? ", which is working there now" : ""}: the two agents see each other's changes.`}
          </p>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="fork-prompt">First message</Label>
          <Textarea
            id="fork-prompt"
            value={prompt}
            placeholder="Optional: what the fork should do"
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) void fork();
            }}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          <Button disabled={working} onClick={() => void fork()}>
            {working ? "Forking…" : "Fork"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
