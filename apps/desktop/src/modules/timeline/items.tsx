import { FilePenIcon, FileTextIcon, ListTodoIcon, SearchIcon, WrenchIcon } from "lucide-react";

import { parseDiff } from "../../state/diff";
import { relativePath } from "../../util/format";
import { CommandTrace, DiffView, ItemTrace, Markdown, ReasoningTrace, StatusMark, ToolTrace } from "../../ui/kit";
import type { ItemRendererProps } from "../../ui/registry";

export const CORE_ITEM_TYPES = [
  "agentMessage",
  "reasoning",
  "plan",
  "command",
  "fileChange",
  "fileRead",
  "search",
  "tool",
] as const;

function summarize(value: unknown): string {
  if (value === null || value === undefined) return "";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 120 ? `${text.slice(0, 120)}…` : text;
}

/** Draws every built-in item type. Registered per type, so any one can be replaced. */
export function CoreItem({ item, thread }: ItemRendererProps) {
  switch (item.type) {
    case "agentMessage":
      return (
        <div className="message agent">
          {item.text ? <Markdown text={item.text} /> : <StatusMark status={item.status} />}
          {item.text && item.status !== "pending" && <StatusMark status={item.status} />}
        </div>
      );
    case "reasoning":
      return <ReasoningTrace text={item.text} status={item.status} />;
    case "plan":
      return (
        <ItemTrace icon={<ListTodoIcon />} title="Plan" status={item.status} open>
          <div className="text-foreground">
            <Markdown text={item.text} />
          </div>
        </ItemTrace>
      );
    case "command":
      return <CommandTrace command={item.command} output={item.output} exitCode={item.exit_code} status={item.status} />;
    case "fileChange": {
      const files = item.diff ? parseDiff(item.diff, item.path) : [];
      const additions = files.reduce((sum, file) => sum + file.additions, 0);
      const deletions = files.reduce((sum, file) => sum + file.deletions, 0);
      const lines = files.reduce((sum, file) => sum + file.lines.length, 0);
      return (
        <ItemTrace
          icon={<FilePenIcon />}
          title={`Edit ${relativePath(item.path, thread.cwd)}`}
          hint={item.path}
          status={item.status}
          description={
            files.length > 0 && (
              <span className="diff-stat">
                <span className="add">+{additions}</span> <span className="del">−{deletions}</span>
              </span>
            )
          }
          open={lines > 0 && lines <= 30}
        >
          {files.length > 0 ? <DiffView files={files} bare /> : null}
        </ItemTrace>
      );
    }
    case "fileRead":
      return (
        <ToolTrace
          icon={<FileTextIcon />}
          title={`Read ${relativePath(item.path, thread.cwd)}`}
          hint={item.path}
          status={item.status}
        />
      );
    case "search":
      return <ToolTrace icon={<SearchIcon />} title={`Search ${item.query}`} hint={item.query} status={item.status} />;
    case "tool":
      return (
        <ToolTrace
          icon={<WrenchIcon />}
          title={
            <>
              {item.name} <span className="font-mono text-xs opacity-70">{summarize(item.input)}</span>
            </>
          }
          hint={item.name}
          status={item.status}
          input={item.input}
          output={item.output}
        />
      );
    case "extension":
      return null;
  }
}

