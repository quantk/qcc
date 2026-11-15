import { useState } from "react";

import type { CommandContribution, ThreadId } from "../../api/protocol.gen";
import { store, useApp } from "../../state/store";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";

/** Ask for a command's parameters, run it, and open the thread it made. */
export function CommandDialog({
  command,
  threadId,
  close,
}: {
  command: CommandContribution;
  threadId: ThreadId | null;
  close: () => void;
}) {
  const thread = useApp((state) => (threadId ? state.threads[threadId] : undefined));
  const [values, setValues] = useState<Record<string, string>>({});
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const missing = command.params.some((param) => param.required && !values[param.name]?.trim());

  const run = async () => {
    setRunning(true);
    setError(null);
    const args = Object.fromEntries(
      Object.entries(values)
        .map(([name, value]) => [name, value.trim()])
        .filter(([, value]) => value !== ""),
    );
    const result = await store().runCommand({
      plugin: command.plugin,
      command: command.id,
      thread_id: command.thread ? threadId : null,
      args,
    });
    setRunning(false);
    if ("error" in result) {
      setError(result.error);
      return;
    }
    close();
    store().notify(result.message);
    if (result.thread_id) store().select(result.thread_id);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void run();
          }}
        >
          <DialogHeader>
            <DialogTitle>{command.title}</DialogTitle>
            <DialogDescription>{command.description}</DialogDescription>
          </DialogHeader>
          <FieldGroup className="gap-4">
            {command.thread && (
              <p className="text-sm">
                <span className="text-muted-foreground">Thread:</span> {thread?.title ?? threadId ?? "none selected"}
              </p>
            )}
            {command.params.map((param, index) => (
              <Field key={param.name}>
                <FieldLabel htmlFor={`param-${param.name}`}>
                  {param.label}
                  {!param.required && <span className="font-normal text-muted-foreground">(optional)</span>}
                </FieldLabel>
                <Input
                  id={`param-${param.name}`}
                  autoFocus={index === 0}
                  value={values[param.name] ?? ""}
                  placeholder={param.placeholder ?? ""}
                  spellCheck={false}
                  onChange={(event) => setValues({ ...values, [param.name]: event.target.value })}
                />
              </Field>
            ))}
            {error && <FieldError>{error}</FieldError>}
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" disabled={running || missing || (command.thread && !threadId)}>
              {running ? "Running…" : "Run"}
            </Button>
          </DialogFooter>
          <p className="text-xs text-muted-foreground">
            From a terminal: <code>qcc {command.plugin} {command.id}</code>
          </p>
        </form>
      </DialogContent>
    </Dialog>
  );
}
