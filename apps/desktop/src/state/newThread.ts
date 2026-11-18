// What the new-thread dialog starts with. The project is never carried over from the last
// thread: the dialog is in a project only when it was opened from one. Directories are
// remembered apart for the two cases, so a thread outside any project does not start in the
// folder of the last project thread, and a project thread starts in the folder last used there.

import type { Project, ProjectId } from "../api/protocol.gen";

/** The choices the dialog remembers between threads. */
export type NewThreadMemory = {
  provider?: string;
  mode?: string;
  model?: string;
  effort?: string;
  /** The directory of the last thread started outside a project. */
  cwd?: string;
  /** The folder last used in each project. */
  folders?: Record<ProjectId, string>;
};

const text = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);

/**
 * The memory read back from storage. Earlier versions saved the last project and its
 * directory under `cwd`; such a directory belonged to that project, so it becomes the
 * project's folder rather than the directory for threads outside projects.
 */
export function readNewThreadMemory(raw: unknown): NewThreadMemory {
  if (!raw || typeof raw !== "object") return {};
  const value = raw as Record<string, unknown>;
  const memory: NewThreadMemory = {
    provider: text(value.provider),
    mode: text(value.mode),
    model: text(value.model),
    effort: text(value.effort),
  };
  const folders: Record<ProjectId, string> = {};
  if (value.folders && typeof value.folders === "object") {
    for (const [project, folder] of Object.entries(value.folders as Record<string, unknown>)) {
      if (typeof folder === "string") folders[project] = folder;
    }
  }
  const legacyProject = text(value.project);
  const cwd = text(value.cwd);
  if (legacyProject) {
    if (cwd && !(legacyProject in folders)) folders[legacyProject] = cwd;
  } else if (cwd) {
    memory.cwd = cwd;
  }
  if (Object.keys(folders).length > 0) memory.folders = folders;
  return memory;
}

/** The project and directory the dialog opens with. */
export function newThreadStart(
  memory: NewThreadMemory,
  preset: ProjectId | undefined,
  projects: readonly Project[],
): { project: ProjectId | null; cwd: string } {
  const project = preset ? projects.find((candidate) => candidate.id === preset) : undefined;
  if (!project) return { project: null, cwd: memory.cwd ?? "" };
  const folder = memory.folders?.[project.id];
  return {
    project: project.id,
    cwd: folder && project.folders.includes(folder) ? folder : (project.folders[0] ?? ""),
  };
}

/** The memory after a thread was started with these choices. */
export function rememberNewThread(
  previous: NewThreadMemory,
  started: {
    provider: string;
    mode: string;
    model: string | null;
    effort: string | null;
    project: Project | undefined;
    cwd: string;
  },
): NewThreadMemory {
  const next: NewThreadMemory = {
    ...previous,
    provider: started.provider,
    mode: started.mode,
    model: started.model ?? undefined,
    effort: started.effort ?? undefined,
  };
  if (!started.project) {
    next.cwd = started.cwd;
  } else if (started.project.folders.includes(started.cwd)) {
    next.folders = { ...previous.folders, [started.project.id]: started.cwd };
  }
  // A one-off directory picked inside a project ("Another directory…") is neither the
  // project's folder nor a choice for threads outside projects, so it is not remembered.
  return next;
}
