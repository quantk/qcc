import { useState } from "react";

import type { McpServerSettings } from "../../api/protocol.gen";
import { type McpDraft, draftOf, mcpNameProblem, renamedMcpServer, serverOf, withMcpServer } from "../../state/mcp";
import { store, useApp } from "../../state/store";
import { EnvEditor } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

/** Add an MCP server (no `name`), or change or remove the one called `name`. */
export function McpServerDialog(props: { close: () => void; name?: string; server?: McpServerSettings }) {
  const { close, name: original, server } = props;
  const [draft, setDraft] = useState<McpDraft>(() => draftOf(original ?? "", server));
  const [saving, setSaving] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const configured = useApp((state) => state.settings?.settings.mcp_servers);
  const taken = Object.keys(configured ?? {}).filter((other) => other !== original);
  const update = (patch: Partial<McpDraft>) => setDraft({ ...draft, ...patch });

  const name = draft.name.trim();
  const nameProblem = mcpNameProblem(name, taken);
  const built = serverOf(draft);
  const problem = nameProblem ?? ("problem" in built ? built.problem : null);

  const save = async () => {
    if (problem || "problem" in built) return;
    setSaving(true);
    const saved = await store().changeSettings((settings) =>
      original ? renamedMcpServer(settings, original, name, built) : withMcpServer(settings, name, built),
    );
    setSaving(false);
    if (saved) close();
  };
  const remove = async () => {
    if (!original) return;
    if (await store().changeSettings((settings) => withMcpServer(settings, original, null))) close();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="sm:max-w-lg">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <DialogHeader>
            <DialogTitle>{original ? `MCP server ${original}` : "New MCP server"}</DialogTitle>
            <DialogDescription>
              Every agent session started from now on gets it, next to the servers of the agent's own configuration.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup className="gap-4">
            <Field>
              <FieldLabel htmlFor="mcp-name">Name</FieldLabel>
              <Input
                id="mcp-name"
                value={draft.name}
                onChange={(event) => update({ name: event.target.value })}
                placeholder="docs"
                spellCheck={false}
                aria-invalid={draft.name !== "" && nameProblem !== null}
                className="font-mono"
              />
              {draft.name !== "" && nameProblem && <FieldDescription className="text-destructive">{nameProblem}</FieldDescription>}
            </Field>
            <Field>
              <FieldLabel>How the agent reaches it</FieldLabel>
              <ToggleGroup
                variant="outline"
                value={[draft.kind]}
                onValueChange={(value: string[]) => value[0] && update({ kind: value[0] as McpDraft["kind"] })}
              >
                <ToggleGroupItem value="stdio">Runs a program</ToggleGroupItem>
                <ToggleGroupItem value="http">HTTP</ToggleGroupItem>
              </ToggleGroup>
            </Field>
            {draft.kind === "stdio" ? (
              <>
                <Field>
                  <FieldLabel htmlFor="mcp-command">Command</FieldLabel>
                  <Input
                    id="mcp-command"
                    value={draft.commandLine}
                    onChange={(event) => update({ commandLine: event.target.value })}
                    placeholder="npx -y @modelcontextprotocol/server-filesystem /path"
                    spellCheck={false}
                    className="font-mono text-xs"
                  />
                  <FieldDescription>
                    The program and its arguments, quoted as in a shell. The agent starts it in the thread's directory.
                  </FieldDescription>
                </Field>
                <Field>
                  <FieldLabel>Environment of the program</FieldLabel>
                  <EnvEditor rows={draft.env} change={(env) => update({ env })} title="A variable the server program sees" />
                </Field>
              </>
            ) : (
              <>
                <Field>
                  <FieldLabel htmlFor="mcp-url">URL</FieldLabel>
                  <Input
                    id="mcp-url"
                    value={draft.url}
                    onChange={(event) => update({ url: event.target.value })}
                    placeholder="https://example.com/mcp"
                    spellCheck={false}
                    className="font-mono text-xs"
                  />
                  <FieldDescription>A streamable HTTP endpoint. opencode gets it only if it supports HTTP servers.</FieldDescription>
                </Field>
                <Field>
                  <FieldLabel>Headers</FieldLabel>
                  <EnvEditor
                    rows={draft.headers}
                    change={(headers) => update({ headers })}
                    addLabel="Add header"
                    namePlaceholder="Authorization"
                    title="A header the agent sends with every request: a token, an API key"
                  />
                </Field>
              </>
            )}
          </FieldGroup>
          <DialogFooter className="sm:justify-between">
            {original ? (
              <Button
                type="button"
                variant={confirmRemove ? "destructive" : "ghost"}
                onClick={() => (confirmRemove ? void remove() : setConfirmRemove(true))}
                title="Sessions started from now on do not get it"
              >
                {confirmRemove ? "Really remove it" : "Remove server"}
              </Button>
            ) : (
              <span />
            )}
            <div className="flex gap-2">
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button type="submit" disabled={saving || problem !== null} title={problem ?? undefined}>
                {saving ? "Saving…" : original ? "Save" : "Add server"}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
