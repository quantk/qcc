import type { Thread } from "../../api/protocol.gen";
import { store } from "../../state/store";
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

/** Asked when archiving would remove uncommitted work. */
export function ArchiveDialog(props: { thread: Thread; refused: string; close: () => void }) {
  return (
    <AlertDialog open onOpenChange={(open) => !open && props.close()}>
      <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>Archive and discard the changes?</AlertDialogTitle>
          <AlertDialogDescription>
            {props.refused}. Archiving removes the thread's working directory. Work that is not committed there is lost.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep the thread</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => void store().archive(props.thread.id, true).then(props.close)}
          >
            Archive anyway
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
