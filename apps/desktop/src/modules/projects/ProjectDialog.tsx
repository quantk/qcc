import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlusIcon, XIcon } from "lucide-react";
import { useState } from "react";

import type { Project } from "../../api/protocol.gen";
import { store } from "../../state/store";
import { tailPath } from "../../util/format";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";

/** Make a project, or change one: its name and its folders. */
export function ProjectDialog(props: {
  close: () => void;
  project?: Project;
  /** What a new project starts with. */
  draft?: { name: string; folders: string[] };
}) {
  const { project, close } = props;
  const [name, setName] = useState(project?.name ?? props.draft?.name ?? "");
  const [folders, setFolders] = useState<string[]>(project?.folders ?? props.draft?.folders ?? []);
  const [typed, setTyped] = useState("");
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const add = (paths: string[]) => {
    setFolders((current) => [...current, ...paths.filter((path) => path && !current.includes(path))]);
  };

  const browse = async () => {
    const picked = await open({ directory: true, multiple: true, title: "Folders of the project" });
    if (Array.isArray(picked)) add(picked);
    else if (typeof picked === "string") add([picked]);
  };

  // A folder typed but not added yet still counts.
  const all = typed.trim() && !folders.includes(typed.trim()) ? [...folders, typed.trim()] : folders;
  const save = async () => {
    setSaving(true);
    const saved = await store().saveProject(project?.id ?? null, name, all);
    setSaving(false);
    if (saved) close();
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
            <DialogTitle>{project ? "Project" : "New project"}</DialogTitle>
            <DialogDescription>
              A project groups threads that work in its repositories and folders. Threads started in one of them join
              it, and so do threads already there that are in no project.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup className="gap-4">
            <Field>
              <FieldLabel htmlFor="project-name">Name</FieldLabel>
              <Input id="project-name" value={name} onChange={(event) => setName(event.target.value)} placeholder="Shop" />
            </Field>
            <Field>
              <FieldLabel htmlFor="project-folder">Repositories and folders</FieldLabel>
              {folders.length > 0 && (
                <ul className="flex flex-col gap-1">
                  {folders.map((folder) => (
                    <li key={folder} className="flex items-center gap-2 rounded-md border px-2.5 py-1 font-mono text-xs">
                      <span className="min-w-0 flex-1 truncate" title={folder}>
                        {tailPath(folder, 64)}
                      </span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-xs"
                        aria-label={`Remove ${folder}`}
                        onClick={() => setFolders(folders.filter((other) => other !== folder))}
                      >
                        <XIcon />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
              <div className="flex gap-2">
                <Input
                  id="project-folder"
                  value={typed}
                  onChange={(event) => setTyped(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" || !typed.trim()) return;
                    event.preventDefault();
                    add([typed.trim()]);
                    setTyped("");
                  }}
                  placeholder="/path/to/repository"
                  spellCheck={false}
                />
                <Button type="button" variant="outline" onClick={() => void browse()}>
                  <FolderPlusIcon /> Browse…
                </Button>
              </div>
            </Field>
          </FieldGroup>
          <DialogFooter className="sm:justify-between">
            {project ? (
              <Button
                type="button"
                variant={confirmDelete ? "destructive" : "ghost"}
                onClick={() => {
                  if (!confirmDelete) return setConfirmDelete(true);
                  void store().deleteProject(project.id).then(close);
                }}
                title="Its threads stay, outside any project; no files are touched"
              >
                {confirmDelete ? "Really delete the project" : "Delete project"}
              </Button>
            ) : (
              <span />
            )}
            <div className="flex gap-2">
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button type="submit" disabled={saving || !name.trim() || all.length === 0}>
                {saving ? "Saving…" : project ? "Save" : "Create project"}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
