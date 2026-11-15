import { useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

export function RenameDialog({ thread, close }: { thread: Thread; close: () => void }) {
  const [title, setTitle] = useState(thread.title ?? "");
  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void store().rename(thread.id, title).then((done) => done && close());
          }}
        >
          <DialogHeader>
            <DialogTitle>Rename thread</DialogTitle>
          </DialogHeader>
          <Input aria-label="Name" value={title} autoFocus onChange={(event) => setTitle(event.target.value)} />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" disabled={!title.trim()}>
              Rename
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
