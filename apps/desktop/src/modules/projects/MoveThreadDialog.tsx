import { useState } from "react";

import type { ThreadId } from "../../api/protocol.gen";
import { store, useApp } from "../../state/store";
import { Choice } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldLabel } from "@/components/ui/field";

/** Put a thread into a project, move it to another, or take it out. */
export function MoveThreadDialog({ threadId, close }: { threadId: ThreadId; close: () => void }) {
  const thread = useApp((state) => state.threads[threadId]);
  const projects = useApp((state) => state.projects);
  const [project, setProject] = useState<string | null>(thread?.project ?? null);

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void store().setProject(threadId, project).then(close);
          }}
        >
          <DialogHeader>
            <DialogTitle>Move to a project</DialogTitle>
          </DialogHeader>
          <Field>
            <FieldLabel>Project</FieldLabel>
            <Choice
              aria-label="Project"
              className="w-full"
              value={project}
              options={[
                { value: null, label: "No project" },
                ...projects.map((candidate) => ({ value: candidate.id, label: candidate.name })),
              ]}
              onChange={setProject}
            />
          </Field>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" disabled={!thread || project === (thread.project ?? null)}>
              Move
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
